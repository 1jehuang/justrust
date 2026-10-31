//! Graph reads of exact encountered hidden representations, never a program map.
use super::*;

/// Consumer-local discovery coordinates, not another source authority. The full
/// existing graph encoding retains owner/site, ordered actuals and all metadata.
#[derive(Clone)]
struct OpaqueIdentity(Arc<[u64]>);

impl OpaqueIdentity {
    fn from_words(words: &[u64], budget: &ir::PlanBudget) -> Result<Self, Error> {
        let bytes = words
            .len()
            .checked_mul(std::mem::size_of::<u64>())
            .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
            .ok_or_else(|| Error::new("opaque discovery coordinate size overflow"))?;
        budget.retain_bytes(bytes).map_err(Error::new)?;
        Ok(Self(Arc::from(words)))
    }

    fn of(opaque: &Arc<ir::OpaqueTy>, budget: &ir::PlanBudget) -> Result<Self, Error> {
        let words = instances::graph_key::encode_opaque(opaque, budget).map_err(Error::new)?;
        Self::from_words(&words, budget)
    }

    fn matches(&self, words: &[u64], budget: &ir::PlanBudget) -> Result<bool, Error> {
        let work = self
            .0
            .len()
            .min(words.len())
            .checked_add(1)
            .ok_or_else(|| Error::new("opaque discovery comparison work overflow"))?;
        budget.charge_work(work).map_err(Error::new)?;
        Ok(self.0.as_ref() == words)
    }
}

fn has_identity(
    identities: &[OpaqueIdentity],
    words: &[u64],
    budget: &ir::PlanBudget,
) -> Result<bool, Error> {
    for identity in identities {
        if identity.matches(words, budget)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn acyclic(
    node: &OpaqueIdentity,
    edges: &[(OpaqueIdentity, Vec<OpaqueIdentity>)],
    path: &mut Vec<OpaqueIdentity>,
    budget: &ir::PlanBudget,
) -> Result<(), Error> {
    budget.check_depth(path.len()).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    if has_identity(path, &node.0, budget)? {
        return Err(Error::new(
            "unsupported: recursive opaque representation dependency",
        ));
    }
    path.push(node.clone());
    for (owner, children) in edges {
        if owner.matches(&node.0, budget)? {
            for child in children {
                acyclic(child, edges, path, budget)?;
            }
            break;
        }
    }
    path.pop();
    Ok(())
}

/// Decode opaque coordinates using only canonical provider templates read from
/// the graph. Each frontier requests every missing provider before awaiting it.
/// Nested owner actuals discover subsequent frontiers without a global join.
pub(super) async fn decode(
    facts: &Facts<'_>,
    words: &[u64],
    types: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<Arc<ir::OpaqueTy>, Error> {
    let mut templates = instances::graph_key::ClosureTemplates::new();
    loop {
        let missing = std::cell::RefCell::new(Vec::new());
        let result = instances::graph_key::decode_opaque_with_closures(
            words,
            types,
            budget,
            &|provider, args| {
                instances::graph_key::replay_closure(
                    types, &templates, &missing, budget, provider, args,
                )
            },
        );
        let mut missing = missing.into_inner();
        if result.is_ok() || missing.is_empty() {
            return result.map_err(Error::new);
        }
        missing.sort();
        missing.dedup();
        let mut pending = Vec::new();
        for provider in missing {
            if templates.contains_key(&provider) {
                return result.map_err(Error::new);
            }
            types
                .stored_closure_owner(provider)
                .map_err(Error::new)?
                .ok_or_else(|| Error::new("closure graph key provider is not a closure"))?;
            let index = types.callable_index(provider).map_err(Error::new)?;
            let handle = facts.cx.request(keys::item(TEMPLATE_BODY, index)).get(0);
            pending.push((provider, handle));
        }
        for (provider, handle) in pending {
            let body: Arc<TemplateBodyFact> = facts.cx.read_fact(handle).await?;
            let closure = body
                .closure
                .as_ref()
                .ok_or_else(|| Error::new("closure template omitted canonical metadata"))?;
            templates.insert(provider, closure.ty.clone());
        }
    }
}

/// Immutable capability assembled from explicit representation-fact reads. It
/// cannot be constructed by a backend or populated from a side cache.
pub(super) struct Representations {
    engine: crate::engine::EngineAuthority,
    records: Arc<[Arc<OpaqueRepresentationFact>]>,
    context: frontend::PlanCtx,
    closures: instances::graph_key::ClosureTemplates,
    normalized: Vec<(Arc<ir::ClosureTy>, Arc<ir::ClosureTy>)>,
}
impl Representations {
    pub(super) fn engine(&self) -> crate::engine::EngineAuthority {
        self.engine
    }
    /// Physical normalization is available only to a consumer that has read the
    /// exact immutable representation facts. This is not a source type-equality
    /// operation and must never be used to discharge public source predicates.
    pub(super) fn normalize(&self, ty: &ir::Ty, budget: &ir::PlanBudget) -> Result<ir::Ty, Error> {
        self.normalize_at(ty, budget, 0)
    }

    fn normalize_at(
        &self,
        ty: &ir::Ty,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<ir::Ty, Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        budget.charge_work(1).map_err(Error::new)?;
        Ok(match ty {
            ir::Ty::Opaque(opaque) => {
                self.normalize_at(self.get(opaque, budget)?.hidden(), budget, depth + 1)?
            }
            ir::Ty::Ref { kind, referent } => ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(self.normalize_at(referent, budget, depth + 1)?),
            },
            ir::Ty::RawPtr { kind, pointee } => ir::Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(self.normalize_at(pointee, budget, depth + 1)?),
            },
            ir::Ty::Array { element, len } => ir::Ty::Array {
                element: Arc::new(self.normalize_at(element, budget, depth + 1)?),
                len: *len,
            },
            ir::Ty::Slice { element } => ir::Ty::Slice {
                element: Arc::new(self.normalize_at(element, budget, depth + 1)?),
            },
            ir::Ty::Tuple(fields) => ir::Ty::Tuple(
                fields
                    .iter()
                    .map(|field| self.normalize_at(field, budget, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            ),
            ir::Ty::Closure(closure) => {
                let physical = self.lookup_closure(closure, budget)?.ok_or_else(|| {
                    Error::new(if self.closures.contains_key(&closure.provider()) {
                        "physical closure changed its canonical source descriptor"
                    } else {
                        "physical closure template was not read through graph handles"
                    })
                })?;
                ir::Ty::Closure(physical)
            }
            ir::Ty::Struct(_) | ir::Ty::Enum(_) | ir::Ty::Dyn(_) => {
                let mut opaque = Vec::new();
                collect(ty, &mut opaque, budget, depth + 1)?;
                if !opaque.is_empty() {
                    return Err(Error::new(
                        "unsupported: opaque nominal or closure representation requires authorized descriptor rebuilding",
                    ));
                }
                ty.clone()
            }
            ir::Ty::AssociatedType(_) | ir::Ty::Param(_) => {
                return Err(Error::new(
                    "physical normalization requires closed declaration actuals",
                ));
            }
            _ => ty.clone(),
        })
    }

    /// Full semantic descriptor matching, including seal, source identities,
    /// every ordered owner actual, signature and capture. Pointer identity is
    /// only a fast path for an already authenticated immutable descriptor.
    fn same_closure(
        &self,
        left: &Arc<ir::ClosureTy>,
        right: &Arc<ir::ClosureTy>,
        budget: &ir::PlanBudget,
    ) -> Result<bool, Error> {
        budget.charge_work(1).map_err(Error::new)?;
        if Arc::ptr_eq(left, right) {
            return Ok(true);
        }
        if left.provider() != right.provider() || left.owner() != right.owner() {
            return Ok(false);
        }
        self.context
            .trait_world()
            .map_err(Error::new)?
            .types_equal(
                &[ir::Ty::Closure(left.clone())],
                &[ir::Ty::Closure(right.clone())],
            )
            .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))
    }

    fn lookup_closure(
        &self,
        closure: &Arc<ir::ClosureTy>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<Arc<ir::ClosureTy>>, Error> {
        for (source, physical) in &self.normalized {
            if self.same_closure(source, closure, budget)? {
                // Unchanged scalar/nonopaque closures keep the exact input
                // descriptor, including independently replayed equal values.
                return Ok(Some(if Arc::ptr_eq(source, physical) {
                    closure.clone()
                } else {
                    physical.clone()
                }));
            }
            if self.same_closure(physical, closure, budget)? {
                return Ok(Some(closure.clone()));
            }
        }
        Ok(None)
    }

    /// Called only while assembling read()'s local capability. Dependencies
    /// are prepared before their owner, and the returned capability never adds
    /// entries or performs substitution during normalize().
    fn prepare_type(
        &mut self,
        ty: &ir::Ty,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        budget.charge_work(1).map_err(Error::new)?;
        match ty {
            ir::Ty::Opaque(opaque) => {
                let hidden = self.get(opaque, budget)?.hidden().clone();
                self.prepare_type(&hidden, budget, depth + 1)?;
            }
            ir::Ty::FnPointer(pointer) => {
                for t in pointer.types() {
                    self.prepare_type(t, budget, depth + 1)?;
                }
            }
            ir::Ty::Closure(closure) => self.prepare_closure(closure, budget, depth + 1)?,
            ir::Ty::Ref { referent, .. } => self.prepare_type(referent, budget, depth + 1)?,
            ir::Ty::RawPtr { pointee, .. } => self.prepare_type(pointee, budget, depth + 1)?,
            ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                self.prepare_type(element, budget, depth + 1)?
            }
            ir::Ty::Tuple(fields) => {
                for ty in fields.iter() {
                    self.prepare_type(ty, budget, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn prepare_closure(
        &mut self,
        closure: &Arc<ir::ClosureTy>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        budget.charge_work(1).map_err(Error::new)?;
        if self.lookup_closure(closure, budget)?.is_some() {
            return Ok(());
        }
        let template = self
            .closures
            .get(&closure.provider())
            .cloned()
            .ok_or_else(|| {
                Error::new("physical closure template was not read through graph handles")
            })?;
        if template.provider() != closure.provider() || template.owner() != closure.owner() {
            return Err(Error::new(
                "physical closure template changed its source identity",
            ));
        }
        let source_owner = self
            .context
            .instance_facts(&ir::CallTarget {
                declaration: template.owner(),
                type_args: Arc::from(closure.owner_args()),
            })
            .map_err(Error::new)?;
        let original = source_owner
            .instantiate_closure_owner(&template, closure.owner_args())
            .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?;
        if !self.same_closure(closure, &original, budget)? {
            return Err(Error::new(
                "physical closure changed its canonical source descriptor",
            ));
        }
        for arg in closure.owner_args() {
            self.prepare_type(arg, budget, depth + 1)?;
        }
        budget
            .charge_work(closure.owner_args().len())
            .map_err(Error::new)?;
        budget
            .retain_bytes(
                closure
                    .owner_args()
                    .len()
                    .checked_mul(std::mem::size_of::<ir::Ty>())
                    .ok_or_else(|| Error::new("physical closure actuals size overflow"))?,
            )
            .map_err(Error::new)?;
        let args = closure
            .owner_args()
            .iter()
            .map(|arg| self.normalize_at(arg, budget, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        let unchanged = source_owner
            .world()
            .types_equal(closure.owner_args(), &args)
            .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?;
        let physical = if unchanged {
            closure.clone()
        } else {
            self.context
                .instance_facts(&ir::CallTarget {
                    declaration: template.owner(),
                    type_args: Arc::from(args.as_slice()),
                })
                .map_err(Error::new)?
                .instantiate_closure_owner(&template, &args)
                .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?
        };
        budget
            .retain_bytes(std::mem::size_of::<(Arc<ir::ClosureTy>, Arc<ir::ClosureTy>)>())
            .map_err(Error::new)?;
        self.normalized.push((closure.clone(), physical));
        Ok(())
    }

    pub(super) fn get(
        &self,
        opaque: &Arc<ir::OpaqueTy>,
        budget: &ir::PlanBudget,
    ) -> Result<&ir::OpaqueHiddenType, Error> {
        let requested = instances::graph_key::encode_opaque(opaque, budget).map_err(Error::new)?;
        for record in self.records.iter() {
            let produced = instances::graph_key::encode_opaque(record.record.opaque(), budget)
                .map_err(Error::new)?;
            if produced == requested {
                return Ok(&record.record);
            }
        }
        Err(Error::new(
            "opaque representation was not read through this instance's graph handles",
        ))
    }
}

/// Every independent fact in a frontier is demanded before any read suspends.
/// Dependencies discovered inside hidden types form subsequent frontiers. The
/// returned capability is local to this consumer, not a shared inventory.
pub(super) async fn read(
    facts: &Facts<'_>,
    roots: &[ir::Ty],
    budget: &ir::PlanBudget,
) -> Result<Representations, Error> {
    let types = facts.types().await?;
    let mut context = types.as_ref().clone();
    let mut pending = Vec::new();
    let mut pending_closures = Vec::new();
    let mut closures = instances::graph_key::ClosureTemplates::new();
    let mut encountered = Vec::new();
    for root in roots {
        collect(root, &mut pending, budget, 0)?;
        collect_closures(root, &mut pending_closures, budget, 0)?;
    }
    let mut demanded = Vec::new();
    let mut records: Vec<Arc<OpaqueRepresentationFact>> = Vec::new();
    let mut edges = Vec::new();
    while !pending.is_empty() || !pending_closures.is_empty() {
        let mut closure_frontier = Vec::new();
        budget
            .retain_bytes(
                pending_closures
                    .len()
                    .checked_mul(std::mem::size_of::<ir::DeclId>())
                    .ok_or_else(|| Error::new("closure frontier size overflow"))?,
            )
            .map_err(Error::new)?;
        budget
            .charge_work(
                pending_closures.len().saturating_mul(
                    pending_closures.len().checked_ilog2().unwrap_or(0) as usize + 1,
                ),
            )
            .map_err(Error::new)?;
        let mut providers = pending_closures
            .iter()
            .map(|closure: &Arc<ir::ClosureTy>| closure.provider())
            .collect::<Vec<_>>();
        encountered.append(&mut pending_closures);
        providers.sort();
        providers.dedup();
        for provider in providers {
            budget.charge_work(1).map_err(Error::new)?;
            if closures.contains_key(&provider) {
                continue;
            }
            types
                .stored_closure_owner(provider)
                .map_err(Error::new)?
                .ok_or_else(|| Error::new("physical closure provider is not a closure"))?;
            let index = types.callable_index(provider).map_err(Error::new)?;
            let handle = facts.cx.request(keys::item(TEMPLATE_BODY, index)).get(0);
            closure_frontier.push((provider, handle));
        }
        let mut frontier = Vec::new();
        for opaque in std::mem::take(&mut pending) {
            let key = instances::graph_key::encode_opaque(&opaque, budget).map_err(Error::new)?;
            if has_identity(&demanded, &key, budget)? {
                continue;
            }
            let identity = OpaqueIdentity::from_words(&key, budget)?;
            let handle = facts
                .cx
                .request(crate::compiler::keys::words(OPAQUE_REPRESENTATION, key)?)
                .get(0);
            let source = types.callable_index(opaque.owner()).map_err(Error::new)?;
            let template = facts
                .cx
                .request(crate::compiler::keys::item(TEMPLATE_BODY, source))
                .get(0);
            budget
                .retain_bytes(std::mem::size_of::<OpaqueIdentity>())
                .map_err(Error::new)?;
            demanded.try_reserve_exact(1).map_err(|_| {
                Error::new("infrastructure: opaque discovery demand allocation failed")
            })?;
            demanded.push(identity.clone());
            let entry = (opaque, identity, handle, template);
            budget
                .retain_bytes(std::mem::size_of_val(&entry))
                .map_err(Error::new)?;
            frontier.try_reserve_exact(1).map_err(|_| {
                Error::new("infrastructure: opaque discovery frontier allocation failed")
            })?;
            frontier.push(entry);
        }
        for (provider, handle) in closure_frontier {
            let body: Arc<TemplateBodyFact> = facts.cx.read_fact(handle).await?;
            let closure = body
                .closure
                .as_ref()
                .ok_or_else(|| Error::new("closure template omitted canonical metadata"))?;
            if closure.ty.provider() != provider {
                return Err(Error::new("physical closure template changed its provider"));
            }
            budget
                .retain_bytes(std::mem::size_of::<(ir::DeclId, Arc<ir::ClosureTy>)>())
                .map_err(Error::new)?;
            closures.insert(provider, closure.ty.clone());
        }
        for (opaque, identity, handle, template) in frontier {
            let record: Arc<OpaqueRepresentationFact> = facts.cx.read_fact(handle).await?;
            let template: Arc<TemplateBodyFact> = facts.cx.read_fact(template).await?;
            let publication = template.publication.as_ref().ok_or_else(|| {
                Error::new("opaque representation lost canonical defining source publication")
            })?;
            let replay = record
                .definition
                .instantiate(&opaque, publication, &mut context, budget)
                .map_err(Error::new)?;
            // Independent canonical substitution may allocate a fresh closure
            // descriptor. Compare its sealed source identity and every ordered
            // actual, not Arc allocation identity, after defining-body replay.
            if !context
                .trait_world()
                .map_err(Error::new)?
                .types_equal(
                    std::slice::from_ref(replay.hidden()),
                    std::slice::from_ref(record.record.hidden()),
                )
                .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?
            {
                return Err(Error::new(
                    "opaque representation changed its defining body witness",
                ));
            }
            if instances::graph_key::encode_opaque(record.record.opaque(), budget)
                .map_err(Error::new)?
                != instances::graph_key::encode_opaque(&opaque, budget).map_err(Error::new)?
            {
                return Err(Error::new(
                    "opaque representation producer changed its requested identity",
                ));
            }
            let mut children = Vec::new();
            collect(record.record.hidden(), &mut children, budget, 0)?;
            collect_closures(record.record.hidden(), &mut pending_closures, budget, 0)?;
            budget
                .retain_bytes(
                    children
                        .len()
                        .checked_mul(std::mem::size_of::<OpaqueIdentity>())
                        .and_then(|bytes| {
                            bytes.checked_add(std::mem::size_of::<(
                                OpaqueIdentity,
                                Vec<OpaqueIdentity>,
                            )>())
                        })
                        .ok_or_else(|| Error::new("opaque discovery edge size overflow"))?,
                )
                .map_err(Error::new)?;
            let mut child_identities = Vec::new();
            child_identities
                .try_reserve_exact(children.len())
                .map_err(|_| {
                    Error::new("infrastructure: opaque discovery children allocation failed")
                })?;
            for child in &children {
                child_identities.push(OpaqueIdentity::of(child, budget)?);
            }
            edges.try_reserve_exact(1).map_err(|_| {
                Error::new("infrastructure: opaque discovery edges allocation failed")
            })?;
            edges.push((identity, child_identities));
            budget
                .retain_bytes(
                    children
                        .len()
                        .checked_mul(std::mem::size_of::<Arc<ir::OpaqueTy>>())
                        .ok_or_else(|| Error::new("opaque discovery pending size overflow"))?,
                )
                .map_err(Error::new)?;
            pending.try_reserve_exact(children.len()).map_err(|_| {
                Error::new("infrastructure: opaque discovery pending allocation failed")
            })?;
            pending.extend(children);
            records.push(record);
        }
    }
    budget
        .retain_bytes(
            demanded
                .len()
                .checked_mul(std::mem::size_of::<OpaqueIdentity>())
                .ok_or_else(|| Error::new("opaque discovery path size overflow"))?,
        )
        .map_err(Error::new)?;
    let mut path = Vec::new();
    path.try_reserve_exact(demanded.len())
        .map_err(|_| Error::new("infrastructure: opaque discovery path allocation failed"))?;
    for root in &demanded {
        acyclic(root, &edges, &mut path, budget)?;
    }
    let mut representations = Representations {
        engine: facts.cx.authority(),
        records: records.into(),
        context,
        closures,
        normalized: Vec::new(),
    };
    for closure in encountered {
        representations.prepare_closure(&closure, budget, 0)?;
    }
    Ok(representations)
}

pub(super) fn contains(ty: &ir::Ty, budget: &ir::PlanBudget) -> Result<bool, Error> {
    fn visit(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        budget.charge_work(1).map_err(Error::new)?;
        match ty {
            ir::Ty::Opaque(_) => Ok(true),
            ir::Ty::Ref { referent, .. } => visit(referent, budget, depth + 1),
            ir::Ty::RawPtr { pointee, .. } => visit(pointee, budget, depth + 1),
            ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                visit(element, budget, depth + 1)
            }
            ir::Ty::Tuple(fields) => {
                for field in fields.iter() {
                    if visit(field, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            ir::Ty::Struct(value) => {
                for actual in value.def().type_args.iter() {
                    if visit(actual, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            ir::Ty::Enum(value) => {
                for actual in value.def().type_args.iter() {
                    if visit(actual, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            ir::Ty::FnPointer(value) => {
                for component in value.types() {
                    if visit(component, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            ir::Ty::Closure(value) => {
                for component in value.types() {
                    if visit(component, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            ir::Ty::Dyn(value) => {
                for component in value.types() {
                    if visit(component, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }
    visit(ty, budget, 0)
}

/// Inventory only closure providers reachable from this consumer's types.
/// Original templates, never mapped descriptors, are later read from the graph.
fn collect_closures(
    ty: &ir::Ty,
    output: &mut Vec<Arc<ir::ClosureTy>>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), Error> {
    budget.check_depth(depth).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    let mut visit = |ty| collect_closures(ty, output, budget, depth + 1);
    match ty {
        ir::Ty::Ref { referent, .. } => visit(referent)?,
        ir::Ty::RawPtr { pointee, .. } => visit(pointee)?,
        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => visit(element)?,
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                visit(field)?;
            }
        }
        ir::Ty::Struct(value) => {
            for arg in value.def().type_args.iter() {
                visit(arg)?;
            }
        }
        ir::Ty::Enum(value) => {
            for arg in value.def().type_args.iter() {
                visit(arg)?;
            }
        }
        ir::Ty::FnItem(value) => {
            for arg in value.type_args.iter() {
                visit(arg)?;
            }
        }
        ir::Ty::Dyn(value) => {
            for ty in value.types() {
                visit(ty)?;
            }
        }
        ir::Ty::Opaque(value) => {
            for arg in value.owner_args() {
                visit(arg)?;
            }
            for bound in value.bounds() {
                for arg in bound.args() {
                    if let ir::OpaqueBoundArg::Type(ty) = arg {
                        visit(ty)?;
                    }
                }
                for (_, ty) in bound.equalities() {
                    visit(ty)?;
                }
            }
        }
        ir::Ty::Closure(value) => {
            budget
                .retain_bytes(std::mem::size_of::<Arc<ir::ClosureTy>>())
                .map_err(Error::new)?;
            for ty in value.types() {
                collect_closures(ty, output, budget, depth + 1)?;
            }
            output.push(value.clone());
        }
        _ => {}
    }
    Ok(())
}

fn collect(
    ty: &ir::Ty,
    output: &mut Vec<Arc<ir::OpaqueTy>>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), Error> {
    budget.check_depth(depth).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    match ty {
        ir::Ty::Opaque(opaque) => {
            // This is occurrence inventory only. The demand frontier dedups
            // complete canonical coordinates, never allocation-local nominal IDs.
            budget
                .retain_bytes(std::mem::size_of::<Arc<ir::OpaqueTy>>())
                .map_err(Error::new)?;
            output.try_reserve_exact(1).map_err(|_| {
                Error::new("infrastructure: opaque discovery inventory allocation failed")
            })?;
            output.push(opaque.clone());
        }
        ir::Ty::Ref { referent, .. } => collect(referent, output, budget, depth + 1)?,
        ir::Ty::RawPtr { pointee, .. } => collect(pointee, output, budget, depth + 1)?,
        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
            collect(element, output, budget, depth + 1)?
        }
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                collect(field, output, budget, depth + 1)?;
            }
        }
        ir::Ty::Struct(value) => {
            for actual in value.def().type_args.iter() {
                collect(actual, output, budget, depth + 1)?;
            }
        }
        ir::Ty::Enum(value) => {
            for actual in value.def().type_args.iter() {
                collect(actual, output, budget, depth + 1)?;
            }
        }
        ir::Ty::FnPointer(value) => {
            for component in value.types() {
                collect(component, output, budget, depth + 1)?;
            }
        }
        ir::Ty::Closure(value) => {
            for component in value.types() {
                collect(component, output, budget, depth + 1)?;
            }
        }
        ir::Ty::FnItem(value) => {
            for component in value.type_args.iter() {
                collect(component, output, budget, depth + 1)?;
            }
        }
        ir::Ty::Dyn(value) => {
            for component in value.types() {
                collect(component, output, budget, depth + 1)?;
            }
        }
        ir::Ty::AssociatedType(_) | ir::Ty::Param(_) => {
            return Err(Error::new(
                "physical representation requires closed source actuals",
            ));
        }
        ir::Ty::Int(_)
        | ir::Ty::Float(_)
        | ir::Ty::Bool
        | ir::Ty::Unit
        | ir::Ty::Never
        | ir::Ty::Char
        | ir::Ty::Str => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_discovery_coordinates_preserve_graph_identity_authority_and_cycles() {
        use crate::compiler::{
            instances::graph_key,
            keys::closed_domain::ClosedDomain,
            memory_layout::domain::compact::{closed, closed_engine_fixture},
        };
        use crate::engine::ItemKey;
        let (engine, budget, plan, root) = closed_engine_fixture(
            "trait Inspect{fn width(&self)->usize;}struct V;impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}struct S;struct R;fn make<T,U>()->impl Inspect{V}fn other<T,U>()->impl Inspect{V}fn main(){let _n=make::<S,R>().width();}",
            "main",
            Arc::from([]),
        );
        let header = engine
            .fact_of::<closed::Header>(&ClosedDomain::header(&root, &budget).unwrap().key(), 0)
            .unwrap();
        let named = |name: &str| {
            (0..plan.callable_count())
                .map(|index| plan.callable_decl(index).unwrap())
                .find(|id| matches!(plan.persistent_decl_path(*id).unwrap().item(), frontend::PersistentDeclItem::Name(found) if found == name))
                .unwrap()
        };
        // The caller retains the exact receiver descriptor from its source
        // occurrence. The independently computed producer header decodes the
        // same canonical coordinates into its own nominal realization. Merely
        // decoding a selected target again would reuse its already canonical IDs.
        let target = header
            .opaque_calls()
            .first()
            .unwrap()
            .owner_target()
            .unwrap();
        assert_eq!(target.declaration, named("make"));
        let words = graph_key::encode(&target, &budget).unwrap();
        let producer = engine
            .fact_of::<closed::Header>(&ClosedDomain::Header(words.clone()).key(), 0)
            .unwrap();
        let decoded = producer.target().clone();
        assert_eq!(graph_key::encode(&decoded, &budget).unwrap(), words);
        assert_ne!(
            target, decoded,
            "fixture must cross genuine graph nominal identities"
        );
        let opaque = |target: &ir::CallTarget| {
            let schema = plan.sema_schema(target.declaration).unwrap();
            let facts = plan.instance_facts(target).unwrap();
            let ty = ir::substitute_ty(
                &schema.signature.result,
                &schema.env,
                &target.type_args,
                ir::FactsCtx::Validated(&facts),
                &mut plan.as_ref().clone(),
                &budget,
            )
            .unwrap();
            let ir::Ty::Opaque(opaque) = ty else {
                panic!("genuine opaque result")
            };
            opaque
        };
        let original = opaque(&target);
        let reconstructed = opaque(&decoded);
        assert_ne!(original, reconstructed);
        let original_key = OpaqueIdentity::of(&original, &budget).unwrap();
        let reconstructed_key = OpaqueIdentity::of(&reconstructed, &budget).unwrap();
        assert!(
            has_identity(
                std::slice::from_ref(&original_key),
                &reconstructed_key.0,
                &budget
            )
            .unwrap()
        );
        let mut reordered_target = target.clone();
        Arc::make_mut(&mut reordered_target.type_args).swap(0, 1);
        let reordered = OpaqueIdentity::of(&opaque(&reordered_target), &budget).unwrap();
        let foreign = OpaqueIdentity::of(
            &opaque(&ir::CallTarget {
                declaration: named("other"),
                type_args: target.type_args.clone(),
            }),
            &budget,
        )
        .unwrap();
        for distinct in [&reordered, &foreign] {
            assert!(
                !has_identity(std::slice::from_ref(&original_key), &distinct.0, &budget).unwrap()
            );
            let edges = vec![
                (reconstructed_key.clone(), vec![distinct.clone()]),
                (distinct.clone(), Vec::new()),
            ];
            acyclic(&original_key, &edges, &mut Vec::with_capacity(2), &budget).unwrap();
        }
        let edges = vec![
            (reconstructed_key.clone(), vec![foreign.clone()]),
            (foreign, vec![reconstructed_key.clone()]),
        ];
        let error =
            acyclic(&original_key, &edges, &mut Vec::with_capacity(2), &budget).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("recursive opaque representation dependency")
        );
        let self_cycle = vec![(reconstructed_key.clone(), vec![reconstructed_key])];
        assert!(
            acyclic(
                &original_key,
                &self_cycle,
                &mut Vec::with_capacity(1),
                &budget
            )
            .is_err()
        );

        // Invalid descriptors remain distinct coordinates, never aliases of a
        // genuine demanded representation. Their later source replay still fails.
        for forged in [
            ir::OpaqueTy::new(
                original.owner(),
                original.owner_args().into(),
                original.site().checked_add(1).unwrap(),
                original.bounds().into(),
                original.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                original.owner(),
                original.owner_args().into(),
                original.site(),
                Arc::from([]),
                original.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                original.owner(),
                original.owner_args().into(),
                original.site(),
                original.bounds().into(),
                Arc::from([ir::SourceRegionId {
                    declaration: original.owner(),
                    index: 0,
                }]),
            ),
        ] {
            let forged = OpaqueIdentity::of(&Arc::new(forged), &budget).unwrap();
            assert!(
                !has_identity(std::slice::from_ref(&original_key), &forged.0, &budget).unwrap()
            );
        }
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        assert!(OpaqueIdentity::from_words(&original_key.0, &denied).is_err());
        let exhausted = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(
            has_identity(
                std::slice::from_ref(&original_key),
                &original_key.0,
                &exhausted
            )
            .is_err()
        );
        let shallow = ir::PlanBudget::new(ir::PlanLimits {
            depth: 0,
            ..Default::default()
        });
        assert!(acyclic(&original_key, &edges, &mut Vec::with_capacity(2), &shallow).is_err());
    }

    #[test]
    fn physical_closure_normalization_rejects_sealed_descriptor_forgery() {
        struct Probe(CompilerProvider);
        impl Provider for Probe {
            fn outputs(&self, key: &Key) -> usize {
                self.0.outputs(key)
            }

            fn start(&self, cx: Context, key: Key) -> TaskFuture {
                if key.kind != 9051 {
                    return self.0.start(cx, key);
                }
                let provider = self.0.clone();
                Box::pin(async move {
                    let facts = Facts { cx: &cx };
                    let types = facts.types().await?;
                    let budget = &provider.0.budget;
                    let opaque = |name| {
                        let declaration = (0..types.callable_count())
                            .map(|index| types.callable_decl(index).unwrap())
                            .find(|id| {
                                matches!(
                                    types.persistent_decl_path(*id).unwrap().item(),
                                    frontend::PersistentDeclItem::Name(found) if found == name
                                )
                            })
                            .unwrap();
                        let result = types.signature(declaration).unwrap().signature.result;
                        assert!(matches!(result, ir::Ty::Opaque(_)));
                        result
                    };
                    let root = opaque("add");
                    let other_root = opaque("other");
                    // Both capabilities come from the real compiler provider,
                    // including defining-body proofs and closure TEMPLATE_BODYs.
                    let mut representations =
                        read(&facts, std::slice::from_ref(&root), budget).await?;
                    let other = read(&facts, std::slice::from_ref(&other_root), budget).await?;
                    let normalized = representations.normalize(&root, budget)?;
                    let ir::Ty::Closure(closure) = &normalized else {
                        panic!("opaque fixture did not normalize to a closure");
                    };
                    assert_eq!(closure.result(), &ir::Ty::Int(ir::IntTy::I32));
                    assert_eq!(closure.captures(), &[ir::Ty::Int(ir::IntTy::I32)]);
                    assert!(representations.normalize(&normalized, budget).is_ok());

                    // Alter the result and capture separately without changing
                    // owner actuals, provider, or the copied original shape seal.
                    let result_index = closure.owner_args().len() + closure.params().len();
                    for changed_index in [result_index, result_index + 1] {
                        let mut index = 0;
                        let forged = Arc::new(
                            closure
                                .try_map::<String>(|ty| {
                                    let changed = index == changed_index;
                                    index += 1;
                                    Ok(if changed { ir::Ty::Bool } else { ty.clone() })
                                })
                                .unwrap(),
                        );
                        assert!(Arc::ptr_eq(
                            closure.shape_seal().unwrap(),
                            forged.shape_seal().unwrap(),
                        ));
                        assert_eq!(forged.owner_args(), closure.owner_args());
                        assert_eq!(forged.provider(), closure.provider());
                        let error = representations
                            .normalize(&ir::Ty::Closure(forged), budget)
                            .expect_err("copied seal sanitized a forged closure descriptor");
                        assert!(
                            error.to_string().contains("canonical source descriptor"),
                            "{error}"
                        );
                    }

                    let foreign = other.normalize(&other_root, budget)?;
                    let ir::Ty::Closure(foreign_closure) = &foreign else {
                        panic!("second opaque fixture did not normalize to a closure");
                    };
                    assert_ne!(closure.provider(), foreign_closure.provider());
                    assert!(
                        representations
                            .normalize(&foreign, budget)
                            .unwrap_err()
                            .to_string()
                            .contains("template was not read through graph handles")
                    );
                    // Even a genuine template read from this graph cannot be
                    // transplanted under a different provider's coordinates.
                    let foreign_template = other
                        .closures
                        .get(&foreign_closure.provider())
                        .unwrap()
                        .clone();
                    let original_template = representations
                        .closures
                        .insert(closure.provider(), foreign_template)
                        .unwrap();
                    let memo = std::mem::take(&mut representations.normalized);
                    assert!(
                        representations
                            .prepare_closure(closure, budget, 0)
                            .unwrap_err()
                            .to_string()
                            .contains("template changed its source identity")
                    );
                    representations
                        .closures
                        .insert(closure.provider(), original_template);
                    representations.normalized = memo;
                    assert!(representations.normalize(&normalized, budget).is_ok());
                    let equivalent =
                        Arc::new(closure.try_map::<String>(|ty| Ok(ty.clone())).unwrap());
                    let entries = representations.normalized.len();
                    let before = budget.usage_snapshot();
                    for _ in 0..128 {
                        let actual = representations
                            .normalize(&ir::Ty::Closure(equivalent.clone()), budget)?;
                        let ir::Ty::Closure(actual) = actual else {
                            unreachable!()
                        };
                        assert!(Arc::ptr_eq(&actual, &equivalent));
                    }
                    let after = budget.usage_snapshot();
                    assert_eq!(
                        before.nodes, after.nodes,
                        "memo lookup repeated owner substitution"
                    );
                    assert_eq!(before.applications, after.applications);
                    assert_eq!(before.obligations, after.obligations);
                    assert_eq!(entries, representations.normalized.len());
                    cx.publish_fact(0, ())
                })
            }
        }

        for workers in [1, 8] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text("fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n} fn other(n:i32)->impl Fn(i32)->i32{move|x|x-n} fn main(){}".into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let probe = engine.request(keys::test_key(9051, vec![]));
            engine.run(workers).unwrap();
            probe.completion().unwrap();
        }
    }
}
