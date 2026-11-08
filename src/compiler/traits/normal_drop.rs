//! Canonical, authority-bound normal-path destruction for the bounded scalar-owner slice.
use super::*;

#[derive(Clone, Debug)]
pub enum NormalDropPlan {
    Trivial,
    /// No negative evidence: requires concrete admission before executable lowering.
    Unresolved,
    Destructor(Arc<ValidatedDestructor>),
}

#[derive(Default)]
pub(super) struct NormalDropScratch {
    context: Option<body_validation::Context>,
    plans: Vec<CompletedNormalDrop>,
    storage: Vec<ir::PlanReservation>,
}

struct CompletedNormalDrop {
    ty: Ty,
    plan: NormalDropPlan,
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
    /// Authenticated native Box pointee glue at its original allocation.
    /// No source destructor is called in addition to this owning recipe.
    BoxValue(Option<Arc<ValidatedDestructor>>),
    /// A finite reference to an exact active canonical owner reached through
    /// an authenticated owning Box edge. Private planner construction binds
    /// this marker to its ancestor descriptor.
    Recursive,
}

struct NormalDropFrame {
    owner: Ty,
    owning_depth: usize,
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
            DropGlue::BoxedObject
            | DropGlue::RcObject
            | DropGlue::RcValue(None)
            | DropGlue::BoxValue(None)
            | DropGlue::Recursive => {}
            DropGlue::RcValue(Some(value)) | DropGlue::BoxValue(Some(value)) => {
                value.collect_targets(out)
            }
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
    /// Dependency discovery before graph field layout exists. Only canonical
    /// user destructor targets escape, never a ValidatedDestructor or glue.
    #[cfg(test)]
    pub(in crate::compiler) fn source_normal_drop_targets(
        &self,
        ty: &Ty,
        scratch: &mut TraitScratch,
    ) -> Result<Vec<CallTarget>> {
        self.world.check_facts(self, true)?;
        let mut targets = Vec::new();
        self.source_normal_drop_targets_inner(
            ty,
            scratch,
            0,
            0,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut targets,
            &mut None,
        )?;
        Ok(targets)
    }

    /// Source-only dependencies, never destructor evidence or triviality.
    pub(in crate::compiler) fn source_normal_drop_dependencies(
        &self,
        ty: &Ty,
        scratch: &mut TraitScratch,
    ) -> Result<(Vec<CallTarget>, Vec<Arc<ir::OpaqueTy>>)> {
        self.world.check_facts(self, true)?;
        let mut targets = Vec::new();
        let mut opaque = Some(Vec::new());
        self.source_normal_drop_targets_inner(
            ty,
            scratch,
            0,
            0,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut targets,
            &mut opaque,
        )?;
        Ok((targets, opaque.unwrap_or_default()))
    }

    #[allow(clippy::too_many_arguments)]
    fn source_normal_drop_targets_inner(
        &self,
        ty: &Ty,
        scratch: &mut TraitScratch,
        depth: usize,
        owning_depth: usize,
        active: &mut Vec<NormalDropFrame>,
        complete: &mut Vec<Ty>,
        targets: &mut Vec<CallTarget>,
        opaque: &mut Option<Vec<Arc<ir::OpaqueTy>>>,
    ) -> Result<()> {
        charged(self.budget(), self.budget().check_depth(depth))?;
        charged(self.budget(), self.budget().charge_work(1))?;
        self.validate_ty_metadata_scratch(ty, self.budget(), scratch)?;
        if matches!(ty, Ty::AssociatedType(_)) {
            let normalized = self.normalize_substituted_projection_type(ty, self.budget())?;
            if self.world.eq_ty(ty, &normalized, 0)? {
                return Err(FactsError::unsupported(
                    self.budget(),
                    "unresolved source cleanup projection",
                ));
            }
            return self.source_normal_drop_targets_inner(
                &normalized,
                scratch,
                depth + 1,
                owning_depth,
                active,
                complete,
                targets,
                opaque,
            );
        }
        if let (Ty::Opaque(descriptor), Some(dependencies)) = (ty, opaque.as_mut()) {
            for previous in dependencies.iter() {
                charged(self.budget(), self.budget().charge_work(1))?;
                if schema_instantiation::same_input(
                    &Ty::Opaque(previous.clone()),
                    ty,
                    self.budget(),
                    0,
                )? {
                    return Ok(());
                }
            }
            push(self.budget(), dependencies, descriptor.clone())?;
            return Ok(());
        }
        if matches!(ty, Ty::Param(_) | Ty::Opaque(_)) {
            return Err(FactsError::unsupported(
                self.budget(),
                "unresolved source cleanup owner",
            ));
        }
        if matches!(
            ty,
            Ty::Ref { .. }
                | Ty::RawPtr { .. }
                | Ty::PhantomData(_)
                | Ty::Cell(_)
                | Ty::ManuallyDrop(_)
        ) {
            return Ok(());
        }
        for frame in active.iter() {
            charged(self.budget(), self.budget().charge_work(1))?;
            if self.world.eq_ty(ty, &frame.owner, 0)? {
                return if owning_depth > frame.owning_depth {
                    Ok(())
                } else {
                    Err(FactsError::invalid(
                        self.budget(),
                        "recursive source cleanup without an owning edge",
                    ))
                };
            }
        }
        for previous in complete.iter() {
            charged(self.budget(), self.budget().charge_work(1))?;
            if schema_instantiation::same_input(previous, ty, self.budget(), 0)? {
                return Ok(());
            }
        }
        push(
            self.budget(),
            active,
            NormalDropFrame {
                owner: ty.clone(),
                owning_depth,
            },
        )?;
        let result: Result<()> = (|| {
            // Full source metadata above authenticates this reserved ownership
            // carrier. It confers an owning edge here, not native layout/glue.
            if let Some((kind, pointee)) = ty.owned_pointer_pointee() {
                if kind == ir::OwnedPointerKind::Box
                    && let Ty::Struct(value) = ty
                    && value.realization().is_none()
                {
                    let source = self
                        .world
                        .schema
                        .native_box_source(value.def().source_decl)?
                        .ok_or_else(|| {
                            FactsError::invalid(
                                self.budget(),
                                "source cleanup Box lost canonical ownership provider",
                            )
                        })?;
                    source
                        .authorize_definition(ty, self.budget())
                        .map_err(|error| FactsError::from_message(self.budget(), &error))?;
                }
                if matches!(pointee, Ty::Dyn(_)) {
                    // Unsizing sites separately demand every vtable target.
                    return Ok(());
                }
                let next = owning_depth.checked_add(1).ok_or_else(|| {
                    FactsError::resource(self.budget(), "source cleanup owning depth overflow")
                })?;
                return self.source_normal_drop_targets_inner(
                    pointee,
                    scratch,
                    depth + 1,
                    next,
                    active,
                    complete,
                    targets,
                    opaque,
                );
            }
            let declaration = match ty {
                Ty::Struct(value) => Some(value.def().source_decl),
                Ty::Enum(value) => Some(value.def().source_decl),
                _ => None,
            };
            let mut has_own_drop = false;
            if let Some(declaration) = declaration {
                let candidates = self
                    .world
                    .trait_impl_index
                    .get(&self.world.catalog.id(CatalogItem::Drop))
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for &position in candidates {
                    charged(self.budget(), self.budget().charge_work(1))?;
                    let implementation = &self.world.headers.impls[position];
                    let owner = match &implementation.trait_ref.self_ty {
                        Ty::Struct(value) => Some(value.def().source_decl),
                        Ty::Enum(value) => Some(value.def().source_decl),
                        _ => None,
                    };
                    has_own_drop |= owner == Some(declaration);
                }
            }
            if has_own_drop {
                let predicate = Predicate::Implements(TraitRef {
                    declaration: self.world.catalog.id(CatalogItem::Drop),
                    self_ty: ty.clone(),
                    args: Arc::from([]),
                });
                let proof = self.world.require_in(
                    &predicate,
                    ProofContext::from_facts(self),
                    scratch.for_world(self.world.id),
                )?;
                let ProofOrigin::Implementation {
                    declaration,
                    arguments,
                } = &proof.0.origin
                else {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "source destructor has no canonical implementation",
                    ));
                };
                let positions = self
                    .world
                    .impl_header_index
                    .get(self.budget(), *declaration)?;
                let mut selected = None;
                for position in positions.iter() {
                    let implementation = &self.world.headers.impls[*position];
                    for (item, body) in implementation.items.iter() {
                        charged(self.budget(), self.budget().charge_work(1))?;
                        if *item == self.world.catalog.id(CatalogItem::DropItem) {
                            if selected.is_some() {
                                return Err(FactsError::invalid(
                                    self.budget(),
                                    "ambiguous source destructor item",
                                ));
                            }
                            self.world.ensure_provider(*body)?;
                            selected = Some(CallTarget {
                                declaration: *body,
                                type_args: arguments.clone(),
                            });
                        }
                    }
                }
                let target = selected.ok_or_else(|| {
                    FactsError::invalid(self.budget(), "source destructor item missing")
                })?;
                // Admission proves entry obligations only. Later executable
                // discharge still authenticates its signature and field layout.
                self.world.instance_facts(&target)?;
                let mut duplicate = false;
                for previous in targets.iter() {
                    charged(self.budget(), self.budget().charge_work(1))?;
                    duplicate |= previous == &target;
                }
                if !duplicate {
                    push(self.budget(), targets, target)?;
                }
            }
            let mut child = |ty: &Ty| {
                self.source_normal_drop_targets_inner(
                    ty,
                    scratch,
                    depth + 1,
                    owning_depth,
                    active,
                    complete,
                    targets,
                    opaque,
                )
            };
            match ty {
                Ty::Struct(value) => {
                    for field in nominal_fields(&value.def().fields, self.budget())?.iter() {
                        child(&field.ty)?;
                    }
                }
                Ty::Enum(value) => {
                    for variant in value.def().variants.iter() {
                        charged(self.budget(), self.budget().charge_work(1))?;
                        for field in nominal_fields(&variant.fields, self.budget())?.iter() {
                            child(&field.ty)?;
                        }
                    }
                }
                Ty::Tuple(parts) => {
                    for part in parts.iter() {
                        child(part)?;
                    }
                }
                Ty::Closure(value) => {
                    for capture in value.captures() {
                        child(capture)?;
                    }
                }
                Ty::Array { element, len } if *len != 0 => child(element)?,
                Ty::DependentArray { .. } | Ty::Slice { .. } => {
                    return Err(FactsError::unsupported(
                        self.budget(),
                        "unsupported: unresolved source cleanup extent",
                    ));
                }
                _ => {}
            }
            Ok(())
        })();
        active.pop();
        result?;
        push(self.budget(), complete, ty.clone())?;
        Ok(())
    }

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
                | Ty::PhantomData(_)
                | Ty::Cell(_)
                | Ty::ManuallyDrop(_)
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
            Ty::DependentArray { element, .. } => {
                let (needs, open) = self.normal_drop_presence(element, depth + 1)?;
                unresolved |= needs || open;
            }
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
        self.normal_drop_plan_with_scratch(ty, &mut TraitScratch::default())
    }
    pub(in crate::compiler) fn normal_drop_plan_with_scratch(
        &self,
        ty: &Ty,
        scratch: &mut TraitScratch,
    ) -> Result<NormalDropPlan> {
        // Live facts, source authority, exact metadata and budget checks must
        // precede even a completed-plan hit. This is the same full admission
        // required by the uncached entry point.
        self.validate_ty_scratch(ty, scratch)?;
        let memo = &mut scratch.normal_drops;
        if !memo
            .context
            .as_ref()
            .map(|context| context.matches(self))
            .transpose()?
            .unwrap_or(false)
        {
            *memo = NormalDropScratch {
                context: Some(body_validation::Context::retain(self)?),
                ..NormalDropScratch::default()
            };
        }
        if !ty.is_nominal_back_edge() {
            for previous in &memo.plans {
                charged(self.budget(), self.budget().charge_work(1))?;
                if schema_instantiation::same_input(&previous.ty, ty, self.budget(), 0)? {
                    return Ok(previous.plan.clone());
                }
            }
        }
        // Recursive glue refers to this root's authenticated DFS ancestors.
        // Never reuse an active or partial subtree from another ancestry.
        let plan = self.normal_drop_plan_inner(ty, 0, 0, &mut Vec::new())?;
        if !ty.is_nominal_back_edge() && memo.plans.len() < MAX_SCRATCH_OBLIGATIONS {
            push_scratch_row(
                self.budget(),
                &mut memo.plans,
                &mut memo.storage,
                CompletedNormalDrop {
                    ty: ty.clone(),
                    plan: plan.clone(),
                },
            )?;
        }
        Ok(plan)
    }
    pub(in crate::compiler) fn native_box_actual_is_closed(&self, actual: &Ty) -> Result<bool> {
        self.world.check_facts(self, true)?;
        self.validate_ty(actual)?;
        let closed = self.world.closedness_of_root(
            actual,
            self.env(),
            TypeUse::Actual,
            0,
            &mut ProofScratch::default(),
        )?;
        Ok(!closed.has_parameters && !closed.has_opaque)
    }
    pub(in crate::compiler) fn native_box_pointee<'a>(&self, ty: &'a Ty) -> Result<Option<&'a Ty>> {
        charged(self.budget(), self.budget().charge_work(1))?;
        let Ty::Struct(owner) = ty else {
            return Ok(None);
        };
        if let Some(read) = owner.realization() {
            read.check_definition(ty)
                .map_err(|error| FactsError::from_message(self.budget(), &error.to_string()))?;
            read.replay(ty, self.world())
                .map_err(|error| FactsError::from_message(self.budget(), &error.to_string()))?;
            if read.is_component() {
                if read.component_pointer_kind() != Some(ir::OwnedPointerKind::Box) {
                    return Ok(None);
                }
            } else {
                let Some(source) = self
                    .world
                    .schema
                    .native_box_source(owner.def().source_decl)?
                else {
                    return Ok(None);
                };
                source
                    .authorize_definition(ty, self.budget())
                    .map_err(|error| FactsError::from_message(self.budget(), &error))?;
            }
        } else {
            if self.phase() == FactsPhase::Universal {
                let Some(source) = self
                    .world
                    .schema
                    .native_box_source(owner.def().source_decl)?
                else {
                    return Ok(None);
                };
                source
                    .authorize_definition(ty, self.budget())
                    .map_err(|error| FactsError::from_message(self.budget(), &error))?;
                let [actual] = owner.def().type_args.as_ref() else {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "native Box has changed actual arguments",
                    ));
                };
                if !self.native_box_actual_is_closed(actual)? {
                    return Ok(None);
                }
            } else if self.phase() != FactsPhase::Closed {
                return Ok(None);
            }
            let Some(application) = self.world.schema.native_box_application(ty, self)? else {
                return Ok(None);
            };
            application
                .authorize_definition(ty, self.budget())
                .map_err(|error| FactsError::from_message(self.budget(), &error))?;
        }
        let [pointee] = owner.def().type_args.as_ref() else {
            return Err(FactsError::invalid(
                self.budget(),
                "native Box has changed actual arguments",
            ));
        };
        Ok(Some(pointee))
    }
    pub(in crate::compiler) fn same_normal_drop_owner(
        &self,
        left: &Ty,
        right: &Ty,
    ) -> Result<bool> {
        charged(self.budget(), self.budget().charge_work(1))?;
        let ((left_id, left_read), (right_id, right_read)) = match (left, right) {
            (Ty::Struct(left), Ty::Struct(right)) => (
                (left.id(), left.realization()),
                (right.id(), right.realization()),
            ),
            (Ty::Enum(left), Ty::Enum(right)) => (
                (left.id(), left.realization()),
                (right.id(), right.realization()),
            ),
            _ => return Ok(false),
        };
        if left_id != right_id {
            return Ok(false);
        }
        if let (Some(left_read), Some(right_read)) = (left_read, right_read) {
            left_read
                .check_definition(left)
                .map_err(|error| FactsError::from_message(self.budget(), &error.to_string()))?;
            left_read
                .replay(left, self.world())
                .map_err(|error| FactsError::from_message(self.budget(), &error.to_string()))?;
            right_read
                .check_definition(right)
                .map_err(|error| FactsError::from_message(self.budget(), &error.to_string()))?;
            right_read
                .replay(right, self.world())
                .map_err(|error| FactsError::from_message(self.budget(), &error.to_string()))?;
            self.validate_ty(left)?;
            self.validate_ty(right)?;
            return self.world.eq_ty(left, right, 0);
        }
        if left_read.is_none()
            && right_read.is_none()
            && !left.is_nominal_back_edge()
            && !right.is_nominal_back_edge()
            && matches!(left_id.index, ir::NominalIndex::SourceSlot(_))
        {
            // SourceSlot is an original source application, not ParentLocal or
            // a detached graph identity. Replay both complete supplied payloads
            // against the original declaration and canonical ordered actuals.
            self.world
                .schema
                .validate_nominal_with_facts(left, self, self.budget())?;
            self.world
                .schema
                .validate_nominal_with_facts(right, self, self.budget())?;
            self.validate_ty(left)?;
            self.validate_ty(right)?;
            return self.world.eq_ty(left, right, 0);
        }
        // Other source representations may share only the exact definition
        // allocation, authenticated in this caller. In particular a weak,
        // parent-local or detached graph ID is not an original source proof.
        if !charged(
            self.budget(),
            ir::identical_type(left, right, self.budget()),
        )? {
            return Ok(false);
        }
        self.validate_ty(left)?;
        self.validate_ty(right)?;
        Ok(true)
    }
    fn normal_drop_plan_inner(
        &self,
        ty: &Ty,
        depth: usize,
        owning_depth: usize,
        active: &mut Vec<NormalDropFrame>,
    ) -> Result<NormalDropPlan> {
        charged(self.budget(), self.budget().check_depth(depth))?;
        charged(self.budget(), self.budget().charge_work(1))?;
        if matches!(ty, Ty::Struct(_) | Ty::Enum(_)) {
            for frame in active.iter() {
                if self.same_normal_drop_owner(ty, &frame.owner)? {
                    if owning_depth <= frame.owning_depth {
                        return Err(FactsError::invalid(
                            self.budget(),
                            "recursive drop without an authenticated owning edge",
                        ));
                    }
                    retain::<ValidatedDestructor>(self.budget(), 1)?;
                    return Ok(NormalDropPlan::Destructor(Arc::new(ValidatedDestructor {
                        owner: frame.owner.clone(),
                        target: None,
                        signature: None,
                        glue: DropGlue::Recursive,
                        world: self.world.clone(),
                        budget: self.budget().clone(),
                    })));
                }
            }
            push(
                self.budget(),
                active,
                NormalDropFrame {
                    owner: ty.clone(),
                    owning_depth,
                },
            )?;
            let result = self.normal_drop_plan_body(ty, depth, owning_depth, active);
            active.pop();
            result
        } else {
            self.normal_drop_plan_body(ty, depth, owning_depth, active)
        }
    }
    fn normal_drop_plan_body(
        &self,
        ty: &Ty,
        depth: usize,
        owning_depth: usize,
        active: &mut Vec<NormalDropFrame>,
    ) -> Result<NormalDropPlan> {
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
                | Ty::PhantomData(_)
                | Ty::Cell(_)
                | Ty::ManuallyDrop(_)
                | Ty::Never
                | Ty::Str
        ) {
            return Ok(NormalDropPlan::Trivial);
        }
        if matches!(ty, Ty::Param(_) | Ty::AssociatedType(_)) {
            return Ok(NormalDropPlan::Unresolved);
        }
        if let Some(pointee) = self.native_box_pointee(ty)?
            && !matches!(pointee, Ty::Dyn(_))
        {
            let next_owning_depth = owning_depth.checked_add(1).ok_or_else(|| {
                FactsError::resource(self.budget(), "owning drop edge depth overflow")
            })?;
            let value =
                match self.normal_drop_plan_inner(pointee, depth + 1, next_owning_depth, active)? {
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
                glue: DropGlue::BoxValue(value),
                world: self.world.clone(),
                budget: self.budget().clone(),
            })));
        }
        if let Some((ir::OwnedPointerKind::Box, pointee)) = ty.owned_pointer_pointee()
            && !matches!(pointee, Ty::Dyn(_))
        {
            if self.phase() == FactsPhase::Universal
                && let Ty::Struct(owner) = ty
                && owner.realization().is_none()
                && let Some(source) = self
                    .world
                    .schema
                    .native_box_source(owner.def().source_decl)?
            {
                source
                    .authorize_definition(ty, self.budget())
                    .map_err(|error| FactsError::from_message(self.budget(), &error))?;
                // Source checking asks a logical ownership question before
                // executable nominal reads exist. Unknown is not trivial and
                // grants no destructor. Closed lowering must recheck through
                // native_box_pointee and obtain genuine read authority.
                return Ok(NormalDropPlan::Unresolved);
            }
            // Legacy spelling/field classification is never executable Box
            // cleanup authority. In particular an empty source Drop body may
            // not silently leak an owner whose component evidence is absent.
            return Err(FactsError::unsupported(
                self.budget(),
                "unsupported: typed Box cleanup requires authenticated native ownership evidence",
            ));
        }
        if let Some((ir::OwnedPointerKind::Rc, pointee)) = ty.owned_pointer_pointee()
            && !matches!(pointee, Ty::Dyn(_))
        {
            let value =
                match self.normal_drop_plan_inner(pointee, depth + 1, owning_depth, active)? {
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
            match self.normal_drop_plan_inner(child, depth + 1, owning_depth, active)? {
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
            Ty::DependentArray { element, .. } => {
                // Entry validation authenticates the declaration-owned usize
                // binder. No extent is guessed: trivial element drop proves
                // trivial array drop for every admissible extent.
                match self.normal_drop_plan_inner(element, depth + 1, owning_depth, active)? {
                    NormalDropPlan::Trivial => DropGlue::None,
                    NormalDropPlan::Unresolved => return Ok(NormalDropPlan::Unresolved),
                    NormalDropPlan::Destructor(_) => {
                        return Err(FactsError::unsupported(
                            self.budget(),
                            "unsupported: dependent array drop glue requires const instantiation",
                        ));
                    }
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
    fn native_recursive_box_plan_is_finite_and_unbound_markers_reject() {
        let prepared = source::prepare_text_with_prelude(
            "recursive-box-authority.rs",
            "enum List{Cons(Box<List>),Nil}fn f(x:List){}fn main(){}",
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap();
        let budget = PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = context.function_decl(0).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            context.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("source body required")
        };
        let facts = publication.facts();
        assert_eq!(facts.phase(), FactsPhase::Closed);
        let checked = publication.source();
        let owner = checked.locals[checked.params[0].0].ty.clone();
        let mut scratch = TraitScratch::default();
        let before = budget.usage_snapshot().work;
        let NormalDropPlan::Destructor(plan) = facts
            .normal_drop_plan_with_scratch(&owner, &mut scratch)
            .unwrap()
        else {
            panic!("recursive owner needs destruction")
        };
        let cold = budget.usage_snapshot().work - before;
        for _ in 0..2 {
            let before = budget.usage_snapshot().work;
            let NormalDropPlan::Destructor(reused) = facts
                .normal_drop_plan_with_scratch(&owner, &mut scratch)
                .unwrap()
            else {
                panic!("completed root lost recursive destruction")
            };
            assert!(Arc::ptr_eq(&plan, &reused));
            assert!(budget.usage_snapshot().work - before < cold);
            assert_eq!(scratch.normal_drops.plans.len(), 1);
        }
        let other = facts
            .world
            .callable_facts(context.function_decl(1).unwrap())
            .unwrap();
        assert!(
            !scratch
                .normal_drops
                .context
                .as_ref()
                .unwrap()
                .matches(&other)
                .unwrap()
        );
        assert!(matches!(
            other
                .normal_drop_plan_with_scratch(&Ty::Unit, &mut scratch)
                .unwrap(),
            NormalDropPlan::Trivial
        ));
        assert!(
            scratch
                .normal_drops
                .context
                .as_ref()
                .unwrap()
                .matches(&other)
                .unwrap()
        );
        let NormalDropPlan::Destructor(rebound) = facts
            .normal_drop_plan_with_scratch(&owner, &mut scratch)
            .unwrap()
        else {
            panic!("original recursive root lost destruction after caller change")
        };
        assert!(!Arc::ptr_eq(&plan, &rebound));
        let foreign_context =
            frontend::PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let foreign = foreign_context
            .callable_facts(foreign_context.function_decl(0).unwrap())
            .unwrap();
        assert!(
            matches!(foreign.normal_drop_plan_with_scratch(&owner, &mut scratch), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        facts
            .normal_drop_plan_with_scratch(&owner, &mut scratch)
            .unwrap();
        let DropGlue::Variants(variants) = plan.glue() else {
            panic!("enum fields")
        };
        let box_drop = &variants[0].1[0].1;
        let DropGlue::BoxValue(Some(recursive)) = box_drop.glue() else {
            panic!("owning Box edge")
        };
        assert!(matches!(recursive.glue(), DropGlue::Recursive));
        assert!(ir::identical_type(recursive.owner(), &owner, &budget).unwrap());
        assert!(recursive.own_target().is_none());
        let Ty::Enum(original) = &owner else {
            panic!("enum owner")
        };
        let replayed = Ty::Enum(ir::EnumTy::new(original.def().clone()));
        assert!(!ir::identical_type(&replayed, &owner, &budget).unwrap());
        assert!(facts.same_normal_drop_owner(&replayed, &owner).unwrap());
        let mut wrong_slot = original.def().clone();
        let ir::NominalIndex::SourceSlot(slot) = wrong_slot.id.index else {
            panic!("source application fixture")
        };
        wrong_slot.id.index = ir::NominalIndex::SourceSlot(slot + 1);
        let wrong_slot = Ty::Enum(ir::EnumTy::new(wrong_slot));
        assert!(!facts.same_normal_drop_owner(&wrong_slot, &owner).unwrap());
        assert!(facts.normal_drop_plan(&wrong_slot).is_err());
        assert!(
            facts
                .normal_drop_plan_with_scratch(&wrong_slot, &mut scratch)
                .is_err()
        );
        let mut changed = original.def().clone();
        changed.variants = Arc::from([]);
        let forged_same_id = Ty::Enum(ir::EnumTy::new(changed));
        assert!(
            facts
                .same_normal_drop_owner(&forged_same_id, &owner)
                .is_err()
        );
        assert!(facts.normal_drop_plan(&forged_same_id).is_err());
        assert!(
            facts
                .normal_drop_plan_with_scratch(&forged_same_id, &mut scratch)
                .is_err()
        );
        plan.validate(&owner, facts).unwrap();
        assert!(recursive.validate(&owner, facts).is_err());
        assert!(plan.targets().is_empty());
        let mut forged = plan.as_ref().clone();
        forged.glue = DropGlue::Recursive;
        assert_eq!(
            forged.validate(&owner, facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
        let mut direct = Vec::new();
        push(
            &budget,
            &mut direct,
            NormalDropFrame {
                owner: owner.clone(),
                owning_depth: 0,
            },
        )
        .unwrap();
        assert_eq!(
            facts
                .normal_drop_plan_inner(&owner, 0, 0, &mut direct)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
        let remaining = budget.limits().work - budget.usage_snapshot().work;
        budget.charge_work(remaining).unwrap();
        assert_eq!(
            facts
                .same_normal_drop_owner(&replayed, &owner)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
        assert_eq!(
            facts.normal_drop_plan(&owner).unwrap_err().kind(),
            FactsErrorKind::Resource
        );
        assert_eq!(
            facts
                .normal_drop_plan_with_scratch(&owner, &mut scratch)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
        let first = budget.first_resource_failure().unwrap();
        assert_eq!(first.kind, ir::ResourceBudgetKind::Work);
        assert_eq!(first.used, budget.limits().work);
        assert!(budget.charge_work(1).is_err());
        assert_eq!(budget.first_resource_failure(), Some(first));
    }

    #[test]
    fn native_source_box_universal_query_defers_and_closed_requires_application_proof() {
        let prepared = source::prepare_text_with_prelude(
            "box-universal-authority.rs",
            "fn generic<T>(x:Box<T>){}fn concrete(x:Box<i32>){}fn main(){}",
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap();
        let budget = PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        for index in [0, 1] {
            let declaration = context.function_decl(index).unwrap();
            let frontend::PublishedCallableBody::Source(publication) =
                context.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("source body required")
            };
            assert_eq!(
                publication.facts().phase(),
                if index == 0 {
                    FactsPhase::Universal
                } else {
                    FactsPhase::Closed
                }
            );
            // Non-generic publication is already a closed body. Ask the
            // canonical world for its original callable context explicitly,
            // rather than relabeling the publication's executable facts.
            let facts = publication
                .facts()
                .world()
                .callable_facts(declaration)
                .unwrap();
            let checked = publication.source();
            let owner = checked.locals[checked.params[0].0].ty.clone();
            let Ty::Struct(value) = &owner else {
                panic!("native Box source expected")
            };
            assert!(value.realization().is_none());
            assert_eq!(facts.phase(), FactsPhase::Universal);
            assert!(
                facts
                    .world
                    .schema
                    .native_box_source(value.def().source_decl)
                    .unwrap()
                    .is_some()
            );
            if index == 0 {
                assert!(
                    !facts
                        .native_box_actual_is_closed(&value.def().type_args[0])
                        .unwrap()
                );
                assert!(facts.native_box_pointee(&owner).unwrap().is_none());
                assert!(matches!(
                    facts.normal_drop_plan(&owner).unwrap(),
                    NormalDropPlan::Unresolved
                ));
            } else {
                assert!(
                    facts
                        .native_box_actual_is_closed(&value.def().type_args[0])
                        .unwrap()
                );
                assert!(matches!(
                    facts.native_box_pointee(&owner).unwrap(),
                    Some(Ty::Int(ir::IntTy::I32))
                ));
                let NormalDropPlan::Destructor(universal) = facts.normal_drop_plan(&owner).unwrap()
                else {
                    panic!("closed actual needs authentic source cleanup even in Universal caller")
                };
                assert!(matches!(universal.glue(), DropGlue::BoxValue(None)));
                universal.validate(&owner, &facts).unwrap();
                assert_eq!(facts.phase(), FactsPhase::Universal);
            }
            if index == 1 {
                let closed = facts
                    .world
                    .instance_facts(&CallTarget {
                        declaration,
                        type_args: Arc::from([]),
                    })
                    .unwrap();
                assert_eq!(closed.phase(), FactsPhase::Closed);
                assert!(matches!(
                    closed.native_box_pointee(&owner).unwrap(),
                    Some(Ty::Int(ir::IntTy::I32))
                ));
                let application = closed
                    .world
                    .schema
                    .native_box_application(&owner, &closed)
                    .unwrap()
                    .unwrap();
                application.authorize_definition(&owner, &budget).unwrap();
                let NormalDropPlan::Destructor(destructor) =
                    closed.normal_drop_plan(&owner).unwrap()
                else {
                    panic!("genuine canonical source application requires owning cleanup")
                };
                assert!(matches!(destructor.glue(), DropGlue::BoxValue(None)));
                assert!(destructor.own_target().is_none());
                destructor.validate(&owner, &closed).unwrap();
                let mut changed = value.def().clone();
                changed.type_args = Arc::from([Ty::Bool]);
                let forged_actual = Ty::Struct(ir::StructTy::new(changed));
                assert!(
                    application
                        .authorize_definition(&forged_actual, &budget)
                        .is_err()
                );
                assert!(closed.native_box_pointee(&forged_actual).is_err());
                let cloned_definition = Ty::Struct(ir::StructTy::new(value.def().clone()));
                assert!(
                    application
                        .authorize_definition(&cloned_definition, &budget)
                        .is_err()
                );
                let mut changed = value.def().clone();
                changed.name.push_str("_forged");
                let forged_name = Ty::Struct(ir::StructTy::new(changed));
                assert!(closed.native_box_pointee(&forged_name).is_err());
            }
        }
    }

    #[test]
    fn typed_box_cleanup_never_uses_reserved_spelling_without_component_evidence() {
        let program =
            frontend::parse("struct Box<T>{ptr:*mut T}struct N;fn f(x:Box<N>){}fn main(){}")
                .unwrap();
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
        let owner = checked.locals[checked.params[0].0].ty.clone();
        assert!(facts.native_box_pointee(&owner).unwrap().is_none());
        assert!(!matches!(
            facts.normal_drop_plan(&owner).unwrap(),
            NormalDropPlan::Destructor(_)
        ));
        let Ty::Struct(value) = &owner else {
            panic!("struct fixture")
        };
        let mut changed = value.def().clone();
        changed.name = "crate::__justrust_std::boxed::Box".into();
        let forged = Ty::Struct(ir::StructTy::new(changed));
        // The legacy classifier recognizes spelling and fields, but this is
        // explicitly not the sealed ownership capability needed by BoxValue.
        assert!(matches!(
            forged.owned_pointer_pointee(),
            Some((ir::OwnedPointerKind::Box, _))
        ));
        assert!(facts.native_box_pointee(&forged).unwrap().is_none());
        assert_eq!(
            facts
                .normal_drop_plan_inner(&forged, 0, 0, &mut Vec::new())
                .unwrap_err()
                .kind(),
            FactsErrorKind::Unsupported
        );
        assert!(facts.normal_drop_plan(&forged).is_err());
        let remaining = budget.limits().work - budget.usage_snapshot().work;
        budget.charge_work(remaining).unwrap();
        assert_eq!(
            facts.native_box_pointee(&owner).unwrap_err().kind(),
            FactsErrorKind::Resource
        );
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Work);
        assert_eq!(failure.used, budget.limits().work);
        assert!(budget.charge_work(1).is_err());
        assert_eq!(budget.first_resource_failure(), Some(failure));
    }

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
        let mut source_scratch = TraitScratch::default();
        for _ in 0..2 {
            assert_eq!(
                facts
                    .source_normal_drop_targets(&owners[1], &mut source_scratch)
                    .unwrap(),
                typed.targets()
            );
            assert!(
                facts
                    .source_normal_drop_targets(&owners[0], &mut source_scratch)
                    .unwrap()
                    .is_empty()
            );
        }
        for (valid, owner) in [(&dynamic, &owners[0]), (&typed, &owners[1])] {
            valid.validate(owner, facts).unwrap();
            for glue in [
                DropGlue::BoxedObject,
                DropGlue::None,
                DropGlue::RcValue(None),
                DropGlue::BoxValue(None),
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
