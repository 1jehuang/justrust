//! Canonical, authority-bound normal-path destruction for the bounded scalar-owner slice.
use super::*;

#[derive(Clone, Debug)]
pub enum NormalDropPlan {
    Trivial,
    /// No negative evidence: requires concrete admission before executable lowering.
    Unresolved,
    Destructor(Arc<ValidatedDestructor>),
}

/// The canonical own-Drop resolution, shared by glue and presence consumers.
/// This is node-local evidence, not a published fact or executable destructor.
enum OwnNormalDrop {
    Trivial,
    Unresolved,
    Resolved(CallTarget, Signature),
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
    /// A shared owning object: decrement its separate count, then dynamically
    /// destroy and free both allocations exactly at the last strong share.
    RcObject,
    /// Sized Rc pointee glue at its original allocation, never a moved snapshot.
    RcValue(Option<Arc<ValidatedDestructor>>),
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
            DropGlue::BoxedObject | DropGlue::RcObject | DropGlue::RcValue(None) => {}
            DropGlue::RcValue(Some(value)) => value.collect_targets(out),
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
    /// Ownership-only presence, never executable glue or negative Copy evidence.
    /// Both flags are accumulated without short circuiting so unresolved families
    /// and failures in later children retain the ordinary planner's behavior.
    pub(in crate::compiler) fn normal_drop_required(
        &self,
        ty: &Ty,
        budget: &PlanBudget,
    ) -> Result<bool> {
        self.validate_ty_with_budget(ty, budget)?;
        let (required, unresolved) = self.normal_drop_presence(ty, 0)?;
        Ok(required && !unresolved)
    }
    fn normal_drop_presence(&self, ty: &Ty, depth: usize) -> Result<(bool, bool)> {
        charged(self.budget(), self.budget().check_depth(depth))?;
        charged(self.budget(), self.budget().charge_work(1))?;
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
            return Ok((false, false));
        }
        if matches!(ty, Ty::Param(_) | Ty::AssociatedType(_)) {
            return Ok((false, true));
        }
        if ty.owned_pointer_object().is_some() {
            return Ok((true, false));
        }
        let read = match ty {
            Ty::Struct(value) => value.realization(),
            Ty::Enum(value) => value.realization(),
            _ => None,
        };
        if let Some(read) = read.filter(|read| read.is_component()) {
            read.check_definition(ty)
                .map_err(|e| FactsError::from_message(self.budget(), &e.to_string()))?;
            // Authenticate every equation and external receipt before using the
            // finite owning-edge inventory. Pending membership is not a proof.
            read.replay(ty, self.world())
                .map_err(|e| FactsError::from_message(self.budget(), &e.to_string()))?;
            return read
                .component_drop_presence(self.budget(), depth, &mut |ty, own_only, depth| {
                    let result = if own_only {
                        if matches!(
                            ty.owned_pointer_pointee(),
                            Some((ir::OwnedPointerKind::Rc, _))
                        ) {
                            Ok((true, false))
                        } else {
                            self.normal_drop_own_presence(ty)
                        }
                    } else {
                        self.normal_drop_presence(ty, depth)
                    };
                    result.map_err(|e| {
                        crate::engine::Error::new(e.into_legacy_diagnostic(self.budget()))
                    })
                })
                .ok_or_else(|| {
                    FactsError::invalid(self.budget(), "missing recursive drop presence capability")
                })?
                .map_err(|e| FactsError::from_message(self.budget(), &e.to_string()));
        }
        if let Some((ir::OwnedPointerKind::Rc, pointee)) = ty.owned_pointer_pointee() {
            let (_, unresolved) = self.normal_drop_presence(pointee, depth + 1)?;
            return Ok((true, unresolved));
        }
        let (mut required, mut unresolved) = self.normal_drop_own_presence(ty)?;
        let mut child = |ty: &Ty| -> Result<()> {
            let (needs, open) = self.normal_drop_presence(ty, depth + 1)?;
            required |= needs;
            unresolved |= open;
            Ok(())
        };
        match ty {
            Ty::Tuple(parts) => {
                for part in parts.iter() {
                    child(part)?;
                }
            }
            Ty::Closure(value) => {
                for part in value.captures() {
                    child(part)?;
                }
            }
            Ty::Array { element, len } => {
                let (needs, open) = self.normal_drop_presence(element, depth + 1)?;
                required |= *len > 0 && needs;
                unresolved |= open;
            }
            Ty::Slice { element } => {
                let (needs, open) = self.normal_drop_presence(element, depth + 1)?;
                if needs && !open {
                    return Err(FactsError::unsupported(
                        self.budget(),
                        "unsupported: drop glue for an unsized slice",
                    ));
                }
                unresolved |= open;
            }
            Ty::Struct(value) => {
                for field in nominal_fields(&value.def().fields, self.budget())?.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, self.budget())?.iter() {
                        child(&field.ty)?;
                    }
                }
            }
            _ => {}
        }
        Ok((required, unresolved))
    }
    fn normal_drop_own_presence(&self, ty: &Ty) -> Result<(bool, bool)> {
        Ok(match self.normal_drop_own(ty)? {
            OwnNormalDrop::Trivial => (false, false),
            OwnNormalDrop::Unresolved => (false, true),
            OwnNormalDrop::Resolved(..) => (true, false),
        })
    }
    fn normal_drop_own(&self, ty: &Ty) -> Result<OwnNormalDrop> {
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
        if implementation.is_none() {
            return Ok(OwnNormalDrop::Trivial);
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
        match resolved.callee() {
            Callee::Direct(target) => Ok(OwnNormalDrop::Resolved(
                target.clone(),
                resolved.signature().clone(),
            )),
            _ if self.phase() == FactsPhase::Universal => Ok(OwnNormalDrop::Unresolved),
            _ => Err(FactsError::unsupported(
                self.budget(),
                "unsupported: unresolved normal Drop",
            )),
        }
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
        if let Some((ir::OwnedPointerKind::Rc, pointee)) = ty.owned_pointer_pointee()
            && !matches!(pointee, Ty::Dyn(_))
        {
            let value = match self.normal_drop_plan_inner(pointee, depth + 1)? {
                NormalDropPlan::Unresolved => return Ok(NormalDropPlan::Unresolved),
                NormalDropPlan::Trivial => None,
                NormalDropPlan::Destructor(value) => Some(value),
            };
            charged(
                self.budget(),
                self.budget()
                    .retain_bytes(std::mem::size_of::<ValidatedDestructor>()),
            )?;
            return Ok(NormalDropPlan::Destructor(Arc::new(ValidatedDestructor {
                owner: ty.clone(),
                target: None,
                signature: None,
                glue: DropGlue::RcValue(value),
                world: self.world.clone(),
                budget: self.budget().clone(),
            })));
        }
        if let Some((owner_kind, _)) = ty.owned_pointer_object() {
            // Erased pointee cleanup uses its exact vtable and the canonical
            // ownership protocol, not the source Drop declaration body.
            charged(
                self.budget(),
                self.budget()
                    .retain_bytes(std::mem::size_of::<ValidatedDestructor>()),
            )?;
            return Ok(NormalDropPlan::Destructor(Arc::new(ValidatedDestructor {
                owner: ty.clone(),
                target: None,
                signature: None,
                glue: match owner_kind {
                    ir::OwnedPointerKind::Box => DropGlue::BoxedObject,
                    ir::OwnedPointerKind::Rc => DropGlue::RcObject,
                },
                world: self.world.clone(),
                budget: self.budget().clone(),
            })));
        }
        let (own, mut unresolved) = match self.normal_drop_own(ty)? {
            OwnNormalDrop::Trivial => (None, false),
            OwnNormalDrop::Unresolved => (None, true),
            OwnNormalDrop::Resolved(target, signature) => (Some((target, signature)), false),
        };
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
            Ty::Tuple(_) | Ty::Closure(_) => {
                let parts = match ty {
                    Ty::Tuple(parts) => parts.as_ref(),
                    Ty::Closure(closure) => closure.captures(),
                    _ => unreachable!(),
                };
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
                for (i, f) in nominal_fields(&s.def().fields, self.budget())?
                    .iter()
                    .enumerate()
                {
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
                    for (i, f) in nominal_fields(&v.fields, self.budget())?.iter().enumerate() {
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

#[cfg(test)]
mod owning_rc_authority_tests {
    use super::*;
    use crate::compiler::{frontend, source};

    #[test]
    fn rc_cleanup_cannot_change_owner_protocol_or_erase_pointee_destructor() {
        let prepared = source::prepare_text_with_prelude(
            "rc-authority.rs",
            "use std::rc::Rc;fn f(x:Rc<dyn Read>,y:Rc<N>,z:Box<dyn Read>){}struct N;trait Read{fn read(&self)->u32;}impl Read for N{fn read(&self)->u32{7}}impl Drop for N{fn drop(&mut self){}}fn main(){}",
            &source::SourceOptions::default(),
        ).unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap();
        let budget = PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let frontend::PublishedCallableBody::Source(publication) = context
            .publish_callable_body(context.function_decl(0).unwrap(), &budget)
            .unwrap()
        else {
            panic!("source body required")
        };
        let facts = publication.facts();
        let checked = publication.source();
        let owners = checked
            .params
            .iter()
            .map(|p| checked.locals[p.0].ty.clone())
            .collect::<Vec<_>>();
        let NormalDropPlan::Destructor(dynamic) = facts.normal_drop_plan(&owners[0]).unwrap()
        else {
            panic!("RcObject required")
        };
        let NormalDropPlan::Destructor(typed) = facts.normal_drop_plan(&owners[1]).unwrap() else {
            panic!("RcValue required")
        };
        assert!(matches!(dynamic.glue(), DropGlue::RcObject));
        assert!(matches!(typed.glue(), DropGlue::RcValue(Some(_))));
        for (valid, owner) in [(&dynamic, &owners[0]), (&typed, &owners[1])] {
            valid.validate(owner, facts).unwrap();
            for glue in [
                DropGlue::BoxedObject,
                DropGlue::None,
                DropGlue::RcValue(None),
            ] {
                let mut forged = valid.as_ref().clone();
                forged.glue = glue;
                assert!(forged.validate(owner, facts).is_err());
            }
            assert!(valid.validate(&owners[2], facts).is_err());
            let mut forged = valid.as_ref().clone();
            forged.budget = PlanBudget::default();
            assert!(forged.validate(owner, facts).is_err());
        }
    }
}
