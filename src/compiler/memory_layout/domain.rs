//! Finite alignment-domain obligations, not closed instance expansion.
//!
//! Each bit names one exact rigid parameter in its owner's ordered environment.
//! Substitution reduces arrays/fields immediately, so recursive calls such as
//! `f::<[T; 2]>()` cannot grow the abstract state beyond those finite bits.
pub(in crate::compiler) mod compact;
use super::super::{ir, source_replay, traits};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Requirement {
    parameters: Vec<bool>,
    raw_pointees: Vec<bool>,
    unsupported: Option<&'static str>,
    deferred_dispatch: bool,
}
impl Requirement {
    fn empty(arity: usize, budget: &ir::PlanBudget) -> Result<Self, String> {
        budget.retain_bytes(
            arity
                .checked_mul(2)
                .ok_or("alignment obligation arity overflow")?,
        )?;
        Ok(Self {
            parameters: vec![false; arity],
            raw_pointees: vec![false; arity],
            unsupported: None,
            deferred_dispatch: false,
        })
    }
    fn merge(&mut self, other: &Self, budget: &ir::PlanBudget) -> Result<bool, String> {
        if self.parameters.len() != other.parameters.len()
            || self.raw_pointees.len() != other.raw_pointees.len()
        {
            return Err("alignment obligation changed canonical owner arity".into());
        }
        let mut changed = false;
        for (left, right) in self
            .parameters
            .iter_mut()
            .zip(&other.parameters)
            .chain(self.raw_pointees.iter_mut().zip(&other.raw_pointees))
        {
            budget.charge_work(1)?;
            if *right && !*left {
                *left = true;
                changed = true;
            }
        }
        if other.deferred_dispatch && !self.deferred_dispatch {
            self.deferred_dispatch = true;
            changed = true;
        }
        if self.unsupported.is_none() && other.unsupported.is_some() {
            self.unsupported = other.unsupported;
            changed = true;
        }
        Ok(changed)
    }
    fn admit(&self) -> Result<(), String> {
        if let Some(reason) = self.unsupported {
            return Err(reason.into());
        }
        Ok(())
    }
}

fn normalize(
    ty: &ir::Ty,
    parameters: &[ir::ParamId],
    out: &mut Requirement,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if let ir::Ty::RawPtr { pointee, .. } = ty {
        return normalize_raw_pointee(pointee, parameters, out, budget);
    }
    if super::super::c_abi::layout::scalar_size_align(ty).is_some() {
        return Ok(());
    }
    match ty {
        ir::Ty::Param(parameter) => {
            let slot = parameters
                .iter()
                .position(|p| p == parameter)
                .ok_or("alignment obligation contains a foreign rigid parameter")?;
            out.parameters[slot] = true;
        }
        ir::Ty::Char | ir::Ty::Ref { .. } => {}
        ir::Ty::RawPtr { pointee, .. } if matches!(pointee.as_ref(), ir::Ty::Dyn(_)) => {}
        ir::Ty::Array { element, .. } => normalize(element, parameters, out, budget, depth + 1)?,
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                normalize(field, parameters, out, budget, depth + 1)?;
            }
        }
        ir::Ty::Struct(structure) => {
            for field in structure.def().fields.read(budget)?.iter() {
                normalize(&field.ty, parameters, out, budget, depth + 1)?;
            }
        }
        ir::Ty::Opaque(_) => {
            out.unsupported = Some(
                "unsupported: opaque target alignment requires graph-authorized physical normalization",
            )
        }
        ir::Ty::AssociatedType(_) => {
            // The original query occurrence must be replayed after exact
            // closed associated-type substitution, never assumed supported.
            out.deferred_dispatch = true;
        }
        ir::Ty::RawPtr { .. } => {
            out.unsupported =
                Some("unsupported: target alignment of an unsized raw pointer is not implemented")
        }
        ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_) => {
            out.unsupported = Some("unsupported: target alignment requires a Sized type")
        }
        ir::Ty::Enum(_) | ir::Ty::FnItem(_) | ir::Ty::Closure(_) | ir::Ty::Never => {
            out.unsupported = Some("unsupported: target alignment for this type is not implemented")
        }
        ir::Ty::Unit | ir::Ty::Bool | ir::Ty::Int(_) | ir::Ty::Float(_) => {
            return Err("target scalar alignment authority is missing".into());
        }
    }
    Ok(())
}

/// The current public observation explicitly excludes raw str/slice pointers.
/// This is separate from AlignmentSupported(T): an enum can be a valid thin
/// raw pointee even while its own target alignment remains unsupported.
fn normalize_raw_pointee(
    ty: &ir::Ty,
    parameters: &[ir::ParamId],
    out: &mut Requirement,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(1)?;
    match ty {
        ir::Ty::Param(parameter) => {
            let slot = parameters
                .iter()
                .position(|p| p == parameter)
                .ok_or("raw alignment obligation contains a foreign rigid parameter")?;
            out.raw_pointees[slot] = true;
        }
        ir::Ty::Str | ir::Ty::Slice { .. } => {
            out.unsupported =
                Some("unsupported: target alignment of an unsized raw pointer is not implemented");
        }
        ir::Ty::AssociatedType(_) => {
            out.deferred_dispatch = true;
        }
        _ => {}
    }
    Ok(())
}

fn may_own_drop(ty: &ir::Ty) -> bool {
    !matches!(
        ty,
        ir::Ty::Int(_)
            | ir::Ty::Float(_)
            | ir::Ty::Bool
            | ir::Ty::Char
            | ir::Ty::Unit
            | ir::Ty::Never
            | ir::Ty::Ref { .. }
            | ir::Ty::RawPtr { .. }
            | ir::Ty::Str
    )
}

/// Original immutable occurrence and the exact canonical selected direct call.
#[derive(Clone)]
struct Edge {
    occurrence: ir::ExprId,
    target: ir::CallTarget,
    proof: traits::DirectCallProof,
    arguments: Vec<(Requirement, Requirement)>,
}

/// Producer-local source proof. Only its compact projection may be published.
#[derive(Clone)]
pub(in crate::compiler) struct Local {
    owner: ir::DeclId,
    facts: Arc<traits::ValidatedFacts>,
    parameters: Arc<[ir::ParamId]>,
    source: Arc<ir::CheckedFunction>,
    publication: Option<Arc<source_replay::CanonicalSourcePublication>>,
    queries: Arc<[(ir::ExprId, ir::TypeAlignmentQuery)]>,
    direct: Requirement,
    edges: Arc<[Edge]>,
    unresolved: Arc<[(ir::ExprId, ir::Callee)]>,
    constructors: Arc<[ir::ExprId]>,
}
impl Local {
    pub(in crate::compiler) fn capture(
        source: Arc<ir::CheckedFunction>,
        publication: Option<Arc<source_replay::CanonicalSourcePublication>>,
        facts: Arc<traits::ValidatedFacts>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        if let Some(publication) = &publication {
            publication
                .validate_source(&source, &facts, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            ir::validate_published_source(publication, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        } else {
            source.validate_with_facts(ir::FactsCtx::Validated(&facts))?;
            ir::validate_call_evidence(&source, &facts, budget)?;
        }
        let parameters: Arc<[ir::ParamId]> = facts.env().ordered_params().collect();
        budget.retain_bytes(std::mem::size_of_val(parameters.as_ref()))?;
        let mut direct = Requirement::empty(parameters.len(), budget)?;
        let mut queries = Vec::new();
        let mut edges = Vec::new();
        let mut unresolved = Vec::new();
        let mut constructors = Vec::new();
        for (index, expression) in source.exprs.iter().enumerate() {
            budget.charge_work(1)?;
            let occurrence = ir::ExprId(index);
            if matches!(
                expression.kind,
                ir::ExprKind::RefToDyn { .. } | ir::ExprKind::OpaqueWrap { .. }
            ) {
                budget.retain_bytes(std::mem::size_of::<ir::ExprId>())?;
                constructors.push(occurrence);
                direct.deferred_dispatch = true;
            }
            if let ir::ExprKind::TypeAlignment(query) = &expression.kind {
                let original = publication
                    .as_ref()
                    .ok_or("alignment query omitted original source publication")?;
                if !Arc::ptr_eq(original.source(), &source) {
                    return Err("alignment query changed original source owner".into());
                }
                query.validate(ir::FactsCtx::Validated(&facts), budget)?;
                if !query.is_private_storage_stride()
                    && !query.is_private_storage_alignment()
                    && !query.is_packed_byte_reserve()
                {
                    normalize(query.queried_type(), &parameters, &mut direct, budget, 0)?;
                }
                budget.retain_bytes(std::mem::size_of::<(ir::ExprId, ir::TypeAlignmentQuery)>())?;
                queries.push((occurrence, query.clone()));
            }
            let ir::ExprKind::Call { target, .. } = &expression.kind else {
                continue;
            };
            let resolved = match target {
                ir::Callee::Direct(target) => ir::Callee::Direct(target.clone()),
                ir::Callee::TraitItem(call) | ir::Callee::Untupled(call) => facts
                    .service()
                    .resolve_trait_call(call, &facts)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?
                    .callee()
                    .clone(),
                ir::Callee::Virtual(_) => target.clone(),
            };
            let ir::Callee::Direct(target) = resolved else {
                budget.retain_bytes(std::mem::size_of::<(ir::ExprId, ir::Callee)>())?;
                unresolved.push((occurrence, target.clone()));
                direct.deferred_dispatch = true;
                continue;
            };
            let proof = facts
                .service()
                .discharge_direct(&target, &facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            let original = facts
                .original_callable_schema(traits::OriginalCallableSubject::Direct(
                    target.declaration,
                ))
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            if original.generic_env().total_arity() != target.type_args.len() {
                return Err("alignment call changed original generic arity".into());
            }
            let mut arguments = Vec::new();
            for actual in target.type_args.iter() {
                let mut requirement = Requirement::empty(parameters.len(), budget)?;
                normalize(actual, &parameters, &mut requirement, budget, 0)?;
                let mut raw = Requirement::empty(parameters.len(), budget)?;
                normalize_raw_pointee(actual, &parameters, &mut raw, budget)?;
                arguments.push((requirement, raw));
            }
            budget.retain_bytes(
                std::mem::size_of::<Edge>()
                    + arguments.len() * std::mem::size_of::<(Requirement, Requirement)>(),
            )?;
            edges.push(Edge {
                occurrence,
                target,
                proof,
                arguments,
            });
        }
        if facts.has_normal_drop_impls() {
            for ty in source
                .locals
                .iter()
                .map(|local| &local.ty)
                .chain(source.exprs.iter().map(|expression| &expression.ty))
                .chain(std::iter::once(&source.result))
            {
                budget.charge_work(1)?;
                if may_own_drop(ty) {
                    direct.deferred_dispatch |= !matches!(
                        facts
                            .normal_drop_plan(ty)
                            .map_err(|e| e.into_legacy_diagnostic(budget))?,
                        traits::NormalDropPlan::Trivial
                    );
                }
            }
        }
        Ok(Self {
            owner: facts.declaration(),
            facts,
            parameters,
            source,
            publication,
            queries: queries.into(),
            direct,
            edges: edges.into(),
            unresolved: unresolved.into(),
            constructors: constructors.into(),
        })
    }
    #[cfg(test)]
    pub(in crate::compiler) fn callees(&self) -> impl Iterator<Item = ir::DeclId> + '_ {
        self.edges.iter().map(|edge| edge.target.declaration)
    }
}

/// Dependency-first SCCs of exactly the graph-read authored call edges.
/// Like nominal realization's topology protocol, traversal uses explicit paid
/// stacks, never host recursion, optimistic backedge success, or item scans.
fn graph_components(
    edges: &[Vec<usize>],
    reverse: &[Vec<usize>],
    budget: &ir::PlanBudget,
) -> Result<Vec<Vec<usize>>, String> {
    let count = edges.len();
    budget.retain_bytes(
        count
            .checked_mul(8 * std::mem::size_of::<usize>() + 2 + std::mem::size_of::<Vec<usize>>())
            .ok_or("alignment SCC storage overflow")?,
    )?;
    let mut seen = vec![false; count];
    let mut order = Vec::with_capacity(count);
    let mut stack = Vec::with_capacity(count);
    for start in 0..count {
        budget.charge_work(1)?;
        if seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push((start, 0usize));
        while let Some((node, edge)) = stack.last_mut() {
            budget.charge_work(1)?;
            if let Some(&target) = edges[*node].get(*edge) {
                *edge += 1;
                if !seen[target] {
                    seen[target] = true;
                    stack.push((target, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    let mut assigned = vec![false; count];
    let mut pending = Vec::with_capacity(count);
    let mut result = Vec::new();
    for start in order.into_iter().rev() {
        budget.charge_work(1)?;
        if assigned[start] {
            continue;
        }
        assigned[start] = true;
        let mut members = Vec::new();
        pending.push(start);
        while let Some(node) = pending.pop() {
            budget.charge_work(1)?;
            members.push(node);
            for &parent in &reverse[node] {
                budget.charge_work(1)?;
                if !assigned[parent] {
                    assigned[parent] = true;
                    pending.push(parent);
                }
            }
        }
        result.push(members);
    }
    result.reverse();
    Ok(result)
}

/// Successful finite fixed point, retaining original source/call/query inputs.
/// Constructible only by replaying the exact graph-read local facts below.
impl Local {
    fn validate(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.charge_work(1)?;
        if !self.facts.budget().same_authority(budget) {
            return Err("alignment local proof has a foreign task budget authority".into());
        }
        let expected = Self::capture(
            self.source.clone(),
            self.publication.clone(),
            self.facts.clone(),
            budget,
        )?;
        if self.owner != expected.owner
            || self.parameters != expected.parameters
            || self.direct != expected.direct
            || self.queries.len() != expected.queries.len()
            || self.edges.len() != expected.edges.len()
            || self.unresolved != expected.unresolved
            || self.constructors != expected.constructors
        {
            return Err(
                "alignment local proof changed exact authored requirements or occurrence inventory"
                    .into(),
            );
        }
        for (edge, original) in self.edges.iter().zip(expected.edges.iter()) {
            budget.charge_work(1)?;
            if edge.occurrence != original.occurrence
                || edge.target != original.target
                || edge.arguments != original.arguments
            {
                return Err("alignment edge changed its original source substitution".into());
            }
        }
        if let Some(publication) = &self.publication {
            if publication.facts().declaration() != self.owner {
                return Err("alignment local proof changed original owner".into());
            }
            publication
                .validate_source(&self.source, publication.facts(), budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        }
        for (occurrence, query) in self.queries.iter() {
            budget.charge_work(1)?;
            let Some(ir::TypedExpr {
                kind: ir::ExprKind::TypeAlignment(original),
                ..
            }) = self.source.exprs.get(occurrence.0)
            else {
                return Err("alignment proof changed original query occurrence".into());
            };
            if !query.same_query(original, budget)? {
                return Err("alignment proof changed canonical query kind or type".into());
            }
        }
        for edge in self.edges.iter() {
            budget.charge_work(1)?;
            if edge.proof.target() != &edge.target
                || !matches!(
                    self.source.exprs.get(edge.occurrence.0).map(|e| &e.kind),
                    Some(ir::ExprKind::Call { .. })
                )
            {
                return Err("alignment proof changed original direct call occurrence".into());
            }
        }
        // Exact unresolved occurrences and object/opaque constructors remain
        // mandatory conditional obligations. Validation authenticates this
        // summary, it does not claim that those obligations have discharged.
        Ok(())
    }
}

/// Finite conditional effects of one producer-selected opaque bound method.
/// Implementation-prefix actuals are reduced at the producer. The method's
/// own binder remains ordered public input, not a fabricated concrete type.
#[derive(Clone)]
pub(in crate::compiler) enum ContractStep {
    Source(ir::CallTarget),
    Forwarded(OpaqueCall),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedSite {
    Call(ir::ExprId),
    Vtable(ir::ExprId),
    CleanupLocal(usize),
    CleanupExpression(ir::ExprId),
    CleanupResult,
}

/// Exact closed source occurrences, selected through canonical proof services.
/// This is a continuation inventory, not successful admission: virtual slots
/// still need their constructor invariant and opaque packing needs its public
/// bound-method contract. Keeping those occurrences prevents a missing target
/// from being represented by an empty call set.
#[derive(Clone)]
pub(in crate::compiler) struct ClosedCalls {
    original: Local,
    target: ir::CallTarget,
    facts: Arc<traits::ValidatedFacts>,
    selected: Arc<[(SelectedSite, ir::CallTarget, traits::DirectCallProof)]>,
    queries: Arc<[(ir::ExprId, ir::Ty)]>,
    virtual_sites: Arc<[ir::ExprId]>,
    opaque_sites: Arc<[OpaqueCall]>,
    packs: Arc<[PackProviders]>,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::compiler) struct OpaqueCall {
    occurrence: ir::ExprId,
    call: ir::TraitCall,
}

impl OpaqueCall {
    pub(in crate::compiler) fn owner_target(&self) -> Result<ir::CallTarget, String> {
        let ir::Ty::Opaque(opaque) = &self.call.trait_ref.self_ty else {
            return Err("opaque alignment call changed public receiver identity".into());
        };
        Ok(ir::CallTarget {
            declaration: opaque.owner(),
            type_args: opaque.owner_args().into(),
        })
    }
}

#[derive(Clone)]
pub(in crate::compiler) struct PackProviders {
    occurrence: ir::ExprId,
    opaque: Arc<ir::OpaqueTy>,
    providers: Arc<[traits::alignment_provider::BoundProvider]>,
}
impl Local {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn closed_calls(
        &self,
        target: &ir::CallTarget,
        facts: Arc<traits::ValidatedFacts>,
        context: &mut super::super::frontend::PlanCtx,
        budget: &ir::PlanBudget,
        engine: Option<crate::engine::EngineAuthority>,
    ) -> Result<ClosedCalls, String> {
        self.validate(budget)?;
        if self.owner != target.declaration
            || facts.target() != Some(target)
            || !Arc::ptr_eq(self.facts.world(), facts.world())
        {
            return Err(
                "closed alignment continuation changed canonical owner or fact subject".into(),
            );
        }
        let env = self.facts.env();
        let mut scratch = traits::TraitScratch::default();
        let mut selected = Vec::new();
        let mut queries = Vec::new();
        let mut virtual_sites = Vec::new();
        let mut opaque_sites = Vec::new();
        let mut packs = Vec::new();
        let mut retain = |occurrence: SelectedSite, next: ir::CallTarget| -> Result<(), String> {
            let proof = facts
                .service()
                .discharge_direct(&next, &facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            budget.retain_bytes(std::mem::size_of::<(
                SelectedSite,
                ir::CallTarget,
                traits::DirectCallProof,
            )>())?;
            selected.push((occurrence, next, proof));
            Ok(())
        };
        for (index, expression) in self.source.exprs.iter().enumerate() {
            budget.charge_work(1)?;
            let occurrence = ir::ExprId(index);
            match &expression.kind {
                ir::ExprKind::TypeAlignment(query)
                    if !query.is_private_storage_stride()
                        && !query.is_private_storage_alignment()
                        && !query.is_packed_byte_reserve() =>
                {
                    query.validate(ir::FactsCtx::Validated(&self.facts), budget)?;
                    let closed = super::super::nominal_realization::substitution::substitute_ty_with_scratch(
                        query.queried_type(), env, &target.type_args, ir::FactsCtx::Validated(&facts),
                        context, budget, engine, &mut scratch,
                    )?;
                    let mut required = Requirement::empty(0, budget)?;
                    normalize(&closed, &[], &mut required, budget, 0)?;
                    required.admit()?;
                    if required.deferred_dispatch {
                        return Err(
                            "unsupported: closed target alignment projection remains unresolved"
                                .into(),
                        );
                    }
                    budget.retain_bytes(std::mem::size_of::<(ir::ExprId, ir::Ty)>())?;
                    queries.push((occurrence, closed));
                }
                ir::ExprKind::Call { target: callee, .. } => {
                    // Hidden dispatch is not a source-level physical query.
                    // Preserve the exact original occurrence for its producer
                    // contract, never operation 140 or a copied hidden type.
                    if matches!(callee, ir::Callee::TraitItem(call)
                        if matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_)))
                    {
                        let resolved = super::super::nominal_realization::substitution::substitute_callee_with_scratch(
                            callee, env, &target.type_args, &facts, context,
                            budget, engine, &mut scratch,
                        )?;
                        let ir::Callee::TraitItem(call) = resolved else {
                            return Err("opaque alignment call lost public source contract".into());
                        };
                        if !matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_)) {
                            return Err(
                                "opaque alignment call changed original opaque receiver".into()
                            );
                        }
                        budget.retain_bytes(std::mem::size_of::<OpaqueCall>())?;
                        opaque_sites.push(OpaqueCall { occurrence, call });
                        continue;
                    }
                    let resolved = super::super::nominal_realization::substitution::substitute_callee_with_scratch(
                        callee, env, &target.type_args, &facts, context,
                        budget, engine, &mut scratch,
                    )?;
                    match resolved {
                        ir::Callee::Direct(next) => retain(SelectedSite::Call(occurrence), next)?,
                        ir::Callee::Untupled(call) => {
                            let next = facts
                                .service()
                                .untupled_target(&call, &facts)
                                .map_err(|e| e.into_legacy_diagnostic(budget))?
                                .target;
                            retain(SelectedSite::Call(occurrence), next)?;
                        }
                        ir::Callee::Virtual(_) => {
                            budget.retain_bytes(std::mem::size_of::<ir::ExprId>())?;
                            virtual_sites.push(occurrence);
                        }
                        ir::Callee::TraitItem(call)
                            if matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_)) =>
                        {
                            budget.retain_bytes(std::mem::size_of::<OpaqueCall>())?;
                            opaque_sites.push(OpaqueCall { occurrence, call });
                        }
                        ir::Callee::TraitItem(_) => return Err(
                            "unsupported: canonical closed alignment dispatch remains unresolved"
                                .into(),
                        ),
                    }
                }
                ir::ExprKind::RefToDyn { reference } => {
                    let operand =
                        self.source.exprs.get(reference.0).ok_or(
                            "alignment vtable constructor operand is outside original source",
                        )?;
                    let concrete = super::super::nominal_realization::substitution::substitute_ty_with_scratch(
                        &operand.ty, env, &target.type_args, ir::FactsCtx::Validated(&facts),
                        context, budget, engine, &mut scratch,
                    )?;
                    let object_ty = super::super::nominal_realization::substitution::substitute_ty_with_scratch(
                        &expression.ty, env, &target.type_args, ir::FactsCtx::Validated(&facts),
                        context, budget, engine, &mut scratch,
                    )?;
                    let object = if let Some((_, object)) = object_ty.owned_pointer_object() {
                        object
                    } else {
                        let ir::Ty::Ref { referent, .. } = &object_ty else {
                            return Err("alignment vtable constructor lost reference output".into());
                        };
                        let ir::Ty::Dyn(object) = referent.as_ref() else {
                            return Err("alignment vtable constructor lost canonical object".into());
                        };
                        object
                    };
                    let referent = match (&concrete, concrete.owned_pointer_pointee()) {
                        (_, Some((_, pointee))) => pointee,
                        (ir::Ty::Ref { referent, .. }, None) => referent.as_ref(),
                        _ => {
                            return Err(
                                "alignment vtable constructor lost concrete referent".into()
                            );
                        }
                    };
                    for next in facts
                        .object_vtable_targets(referent, object)
                        .map_err(|e| e.into_legacy_diagnostic(budget))?
                    {
                        retain(SelectedSite::Vtable(occurrence), next)?;
                    }
                    if facts.has_normal_drop_impls() {
                        match facts.normal_drop_plan_with_scratch(referent, &mut scratch)
                            .map_err(|e| e.into_legacy_diagnostic(budget))?
                        {
                            traits::NormalDropPlan::Destructor(drop) => {
                                for next in drop.targets() {
                                    retain(SelectedSite::Vtable(occurrence), next.clone())?;
                                }
                            }
                            traits::NormalDropPlan::Unresolved => return Err(
                                "unsupported: alignment object constructor cleanup remains unresolved".into()),
                            traits::NormalDropPlan::Trivial => {}
                        }
                    }
                }
                ir::ExprKind::OpaqueWrap { opaque, value } => {
                    let authored = self
                        .source
                        .exprs
                        .get(value.0)
                        .ok_or("opaque alignment contract lost original packing value")?;
                    let public = super::super::nominal_realization::substitution::substitute_ty_with_scratch(
                        &ir::Ty::Opaque(opaque.clone()), env, &target.type_args,
                        ir::FactsCtx::Validated(&facts), context, budget, engine, &mut scratch,
                    )?;
                    let ir::Ty::Opaque(public) = public else {
                        return Err("opaque alignment packing changed public type identity".into());
                    };
                    let authored_value = super::super::nominal_realization::substitution::substitute_ty_with_scratch(
                        &authored.ty, env, &target.type_args, ir::FactsCtx::Validated(&facts),
                        context, budget, engine, &mut scratch,
                    )?;
                    let providers = traits::alignment_provider::bound_providers(
                        facts.clone(),
                        &public,
                        &authored_value,
                    )
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
                    // The original authored value type is used only inside its
                    // source owner. No opaque representation/layout operation
                    // or caller-visible concrete type is demanded or published.
                    budget.retain_bytes(std::mem::size_of::<PackProviders>())?;
                    packs.push(PackProviders {
                        occurrence,
                        opaque: public,
                        providers: providers.into(),
                    });
                    if facts.has_normal_drop_impls() && !matches!(authored_value, ir::Ty::Opaque(_))
                    {
                        match facts
                            .normal_drop_plan_with_scratch(&authored_value, &mut scratch)
                            .map_err(|e| e.into_legacy_diagnostic(budget))?
                        {
                            traits::NormalDropPlan::Destructor(drop) => {
                                for next in drop.targets() {
                                    retain(
                                        SelectedSite::CleanupExpression(occurrence),
                                        next.clone(),
                                    )?;
                                }
                            }
                            traits::NormalDropPlan::Unresolved => return Err(
                                "unsupported: original opaque producer cleanup remains unresolved"
                                    .into(),
                            ),
                            traits::NormalDropPlan::Trivial => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if facts.has_normal_drop_impls() {
            for (site, ty) in self
                .source
                .locals
                .iter()
                .enumerate()
                .map(|(index, local)| (SelectedSite::CleanupLocal(index), &local.ty))
                .chain(
                    self.source
                        .exprs
                        .iter()
                        .enumerate()
                        .map(|(index, expression)| {
                            (
                                SelectedSite::CleanupExpression(ir::ExprId(index)),
                                &expression.ty,
                            )
                        }),
                )
                .chain(std::iter::once((
                    SelectedSite::CleanupResult,
                    &self.source.result,
                )))
            {
                budget.charge_work(1)?;
                if !may_own_drop(ty) {
                    continue;
                }
                let closed =
                    super::super::nominal_realization::substitution::substitute_ty_with_scratch(
                        ty,
                        env,
                        &target.type_args,
                        ir::FactsCtx::Validated(&facts),
                        context,
                        budget,
                        engine,
                        &mut scratch,
                    )?;
                match facts
                    .normal_drop_plan_with_scratch(&closed, &mut scratch)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?
                {
                    traits::NormalDropPlan::Destructor(drop) => {
                        for next in drop.targets() {
                            retain(site, next.clone())?;
                        }
                    }
                    traits::NormalDropPlan::Unresolved => {
                        return Err(
                            "unsupported: closed alignment cleanup remains unresolved".into()
                        );
                    }
                    traits::NormalDropPlan::Trivial => {}
                }
            }
        }
        Ok(ClosedCalls {
            original: self.clone(),
            target: target.clone(),
            facts,
            selected: selected.into(),
            queries: queries.into(),
            virtual_sites: virtual_sites.into(),
            opaque_sites: opaque_sites.into(),
            packs: packs.into(),
        })
    }
}
impl ClosedCalls {
    pub(in crate::compiler) fn provider_declarations(
        &self,
    ) -> impl Iterator<Item = ir::DeclId> + '_ {
        self.packs
            .iter()
            .flat_map(|pack| pack.providers.iter())
            .filter_map(|provider| match provider {
                traits::alignment_provider::BoundProvider::Concrete { seed, .. } => {
                    Some(seed.declaration())
                }
                traits::alignment_provider::BoundProvider::Forwarded { .. } => None,
            })
    }
    /// Validation authenticates a continuation, not successful admission.
    /// Replaying the entire occurrence inventory detects a removed selected
    /// target, erased constructor, changed actual argument or virtual slot.
    pub(in crate::compiler) fn validate(
        &self,
        context: &mut super::super::frontend::PlanCtx,
        budget: &ir::PlanBudget,
        engine: Option<crate::engine::EngineAuthority>,
    ) -> Result<(), String> {
        let replay = self.original.closed_calls(
            &self.target,
            self.facts.clone(),
            context,
            budget,
            engine,
        )?;
        if self.selected.len() != replay.selected.len()
            || self.queries != replay.queries
            || self.virtual_sites != replay.virtual_sites
            || self.opaque_sites != replay.opaque_sites
            || self.packs.len() != replay.packs.len()
        {
            return Err("closed alignment inventory changed original occurrence coverage".into());
        }
        for (pack, expected) in self.packs.iter().zip(replay.packs.iter()) {
            if pack.occurrence != expected.occurrence
                || pack.opaque != expected.opaque
                || pack.providers.len() != expected.providers.len()
            {
                return Err(
                    "opaque alignment contract changed original source packing inventory".into(),
                );
            }
            for (provider, original) in pack.providers.iter().zip(expected.providers.iter()) {
                provider
                    .validate_same(original, &self.facts)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
            }
        }
        for ((occurrence, target, proof), (original, expected, _)) in
            self.selected.iter().zip(replay.selected.iter())
        {
            budget.charge_work(1)?;
            if occurrence != original || target != expected || proof.target() != target {
                return Err("closed alignment inventory changed canonical target proof".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{frontend, source};

    fn fixture(text: &str, root_name: &str) -> (Vec<Local>, ir::DeclId, ir::PlanBudget) {
        let (locals, root, budget, _) = fixture_plan(text, root_name);
        (locals, root, budget)
    }
    pub(super) fn fixture_plan(
        text: &str,
        root_name: &str,
    ) -> (Vec<Local>, ir::DeclId, ir::PlanBudget, frontend::PlanCtx) {
        let options = source::SourceOptions::default();
        let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
        let mut assembler = source::SourceAssembler::new(&options);
        let mut step = assembler
            .provide(Arc::new(source::prepare_text_unit(
                "alignment-domain.rs",
                text,
                &options,
            )))
            .unwrap();
        let prepared = loop {
            match step {
                source::AssemblyStep::Done(prepared) => break prepared,
                source::AssemblyStep::Need(need) => {
                    let unit = match need.parent.as_deref() {
                        None => source::prepare_prelude_unit(&options, &expansion),
                        Some(parent) => {
                            source::prepare_child_unit(parent, need.index, &options, &expansion)
                        }
                    };
                    step = assembler.provide(Arc::new(unit)).unwrap();
                }
            }
        };
        let program = frontend::resolve(&frontend::parse_prepared(&prepared).unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let index = program
            .functions
            .iter()
            .position(|function| {
                function
                    .name
                    .segments()
                    .last()
                    .is_some_and(|id| program.names.spelling(*id).unwrap() == root_name)
            })
            .unwrap();
        let root = plan.function_decl(index).unwrap();
        let locals = fixture_locals(&plan, root, &budget);
        (locals, root, budget, plan)
    }
    pub(super) fn fixture_locals(
        plan: &frontend::PlanCtx,
        root: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Vec<Local> {
        let mut pending = vec![root];
        let mut visited = std::collections::HashSet::new();
        let mut locals = Vec::new();
        while let Some(owner) = pending.pop() {
            if !visited.insert(owner) {
                continue;
            }
            let (checked, publication, facts) =
                match plan.publish_callable_body(owner, budget).unwrap() {
                    frontend::PublishedCallableBody::Source(publication) => (
                        publication.source().clone(),
                        Some(publication.clone()),
                        publication.facts().clone(),
                    ),
                    frontend::PublishedCallableBody::Recipe(checked) => {
                        (checked, None, plan.callable_facts(owner).unwrap())
                    }
                };
            let local = Local::capture(checked, publication, facts, budget).unwrap();
            pending.extend(local.callees());
            locals.push(local);
        }
        locals
    }
    #[test]
    fn alignment_domain_original_query_kind_bits_and_call_arguments_are_sealed() {
        let (locals, root, budget) = fixture(
            "fn a<T>()->usize{std::mem::align_of::<[T;2]>()}fn main(){}",
            "a",
        );
        let owner = locals
            .iter()
            .find(|local| !local.queries.is_empty())
            .unwrap();
        owner.validate(&budget).unwrap();
        let mut bits = owner.clone();
        bits.direct.parameters[0] = false;
        assert!(bits.validate(&budget).is_err());
        let mut kind = owner.clone();
        let mut queries = kind.queries.to_vec();
        queries[0].1 = ir::TypeAlignmentQuery::issue_kind(
            ir::Ty::Param(kind.parameters[0]),
            &kind.facts,
            ir::TypeQueryKind::PrivateStorageAlignment,
        )
        .unwrap();
        kind.queries = queries.into();
        assert!(kind.validate(&budget).is_err());
        let mut caller = locals
            .iter()
            .find(|local| local.owner == root)
            .unwrap()
            .clone();
        let mut edges = caller.edges.to_vec();
        edges[0].arguments[0].0.parameters[0] = false;
        caller.edges = edges.into();
        assert!(caller.validate(&budget).is_err());
        let depth = ir::PlanBudget::new(ir::PlanLimits {
            depth: 0,
            ..Default::default()
        });
        let mut requirement = Requirement::empty(owner.parameters.len(), &depth).unwrap();
        let array = ir::Ty::Array {
            element: Arc::new(ir::Ty::Param(owner.parameters[0])),
            len: 2,
        };
        assert!(normalize(&array, &owner.parameters, &mut requirement, &depth, 0).is_err());
    }

    #[test]
    fn alignment_domain_closed_selected_targets_and_virtual_sites_remain_authenticated() {
        for (source, root_name, actuals) in [
            (
                "trait Step{fn step(self)->usize;}impl Step for u16{fn step(self)->usize{std::mem::align_of::<u16>()}}fn invoke<T:Step>(x:T)->usize{x.step()}fn main(){}",
                "invoke",
                vec![ir::Ty::Int(ir::IntTy::U16)],
            ),
            (
                "trait Inspect{fn width(&self)->usize;}fn invoke(x:&dyn Inspect)->usize{x.width()}fn main(){}",
                "invoke",
                vec![],
            ),
        ] {
            let (locals, owner, budget, mut plan) = fixture_plan(source, root_name);
            let local = locals.iter().find(|local| local.owner == owner).unwrap();
            if !local.unresolved.is_empty() {
                let mut changed = local.clone();
                changed.unresolved = Arc::from([]);
                assert!(changed.validate(&budget).is_err());
            }
            let target = ir::CallTarget {
                declaration: owner,
                type_args: actuals.into(),
            };
            let facts = plan.instance_facts(&target).unwrap();
            let calls = local
                .closed_calls(&target, facts, &mut plan, &budget, None)
                .unwrap();
            calls.validate(&mut plan, &budget, None).unwrap();
            let mut erased = calls.clone();
            if !erased.selected.is_empty() {
                erased.selected = Arc::from([]);
            } else {
                assert!(!erased.virtual_sites.is_empty());
                erased.virtual_sites = Arc::from([]);
            }
            assert!(erased.validate(&mut plan, &budget, None).is_err());
            let mut changed = calls;
            changed.target.declaration.index += 1;
            assert!(changed.validate(&mut plan, &budget, None).is_err());
        }
    }

    #[test]
    fn alignment_domain_opaque_pack_keeps_original_provider_inventory() {
        let (locals, owner, budget, mut plan) = fixture_plan(
            "struct V;trait Inspect{fn width(&self)->usize;}impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn make()->impl Inspect{V}fn main(){}",
            "make",
        );
        let local = locals.iter().find(|local| local.owner == owner).unwrap();
        let target = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([]),
        };
        let facts = plan.instance_facts(&target).unwrap();
        let calls = local
            .closed_calls(&target, facts, &mut plan, &budget, None)
            .unwrap();
        calls.validate(&mut plan, &budget, None).unwrap();
        assert_eq!(calls.packs.len(), 1);
        assert_eq!(calls.packs[0].providers.len(), 1);
        let traits::alignment_provider::BoundProvider::Concrete { public, seed } =
            &calls.packs[0].providers[0]
        else {
            panic!("authentic concrete pack became forwarded")
        };
        assert!(
            matches!(&public.self_ty, ir::Ty::Opaque(opaque) if opaque == &calls.packs[0].opaque)
        );
        seed.validate().unwrap();
        let mut erased = calls.clone();
        erased.packs = Arc::from([]);
        assert!(erased.validate(&mut plan, &budget, None).is_err());
        let mut erased = calls;
        let mut pack = erased.packs[0].clone();
        pack.providers = Arc::from([]);
        erased.packs = Arc::from([pack]);
        assert!(erased.validate(&mut plan, &budget, None).is_err());
    }

    #[test]
    fn alignment_domain_raw_metadata_and_unused_unsupported_are_preserved() {
        let (_, root, budget, plan) = fixture_plan(
            "fn raw<T:?Sized>()->usize{std::mem::align_of::<*const T>()}fn main(){}",
            "raw",
        );
        let (parameters, raw_pointees) = compact::tests::source_requirement(&plan, root, &budget);
        assert_eq!(parameters, [false]);
        assert_eq!(raw_pointees, [true]);
        compact::tests::assert_propagated_raw_requirement_is_sealed(&plan, root, &budget);
        compact::tests::source_summary(&plan, root, &budget)
            .admit_symbolic(root)
            .unwrap();
        for (source, expected) in [
            (
                "fn raw<T:?Sized>()->usize{std::mem::align_of::<*const T>()}fn unused()->usize{raw::<str>()}fn main(){}",
                "unsized raw pointer",
            ),
            (
                "enum E{A,B}fn wrap<T>()->usize{std::mem::align_of::<[T;2]>()}fn unused()->usize{wrap::<E>()}fn main(){}",
                "unsupported: target alignment",
            ),
        ] {
            let (_, root, budget, plan) = fixture_plan(source, "unused");
            let summary = compact::tests::source_summary(&plan, root, &budget);
            let error = summary.admit_symbolic(root).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }
}
