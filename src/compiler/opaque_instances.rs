//! Graph reads of exact encountered hidden representations, never a program map.
use super::*;

/// Immutable capability assembled from explicit representation-fact reads. It
/// cannot be constructed by a backend or populated from a side cache.
pub(super) struct Representations {
    records: Arc<[Arc<OpaqueRepresentationFact>]>,
}
impl Representations {
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
                self.normalize_at(self.get(opaque)?.hidden(), budget, depth + 1)?
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
            ir::Ty::Struct(_) | ir::Ty::Enum(_) | ir::Ty::Closure(_) | ir::Ty::Dyn(_) => {
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

    pub(super) fn get(&self, opaque: &Arc<ir::OpaqueTy>) -> Result<&ir::OpaqueHiddenType, Error> {
        self.records
            .iter()
            .find(|record| record.record.opaque() == opaque)
            .map(|record| &record.record)
            .ok_or_else(|| {
                Error::new(
                    "opaque representation was not read through this instance's graph handles",
                )
            })
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
    for root in roots {
        collect(root, &mut pending, budget, 0)?;
    }
    let mut demanded = Vec::new();
    let mut records: Vec<Arc<OpaqueRepresentationFact>> = Vec::new();
    let mut edges = Vec::new();
    while !pending.is_empty() {
        let mut frontier = Vec::new();
        for opaque in std::mem::take(&mut pending) {
            if demanded.contains(&opaque) {
                continue;
            }
            budget.charge_work(demanded.len()).map_err(Error::new)?;
            let key = instances::graph_key::encode_opaque(&opaque, budget).map_err(Error::new)?;
            let handle = facts
                .cx
                .request(Key::new(OPAQUE_REPRESENTATION, key))
                .get(0);
            let source = types.callable_index(opaque.owner()).map_err(Error::new)?;
            let template = facts
                .cx
                .request(crate::compiler::keys::item(TEMPLATE_BODY, source))
                .get(0);
            demanded.push(opaque.clone());
            frontier.push((opaque, handle, template));
        }
        for (opaque, handle, template) in frontier {
            let record: Arc<OpaqueRepresentationFact> = facts.cx.read_fact(handle).await?;
            let template: Arc<TemplateBodyFact> = facts.cx.read_fact(template).await?;
            let publication = template.publication.as_ref().ok_or_else(|| {
                Error::new("opaque representation lost canonical defining source publication")
            })?;
            let replay = record
                .definition
                .instantiate(&opaque, publication, &mut context, budget)
                .map_err(Error::new)?;
            if !ir::identical_type(replay.hidden(), record.record.hidden(), budget)
                .map_err(Error::new)?
            {
                return Err(Error::new(
                    "opaque representation changed its defining body witness",
                ));
            }
            if record.record.opaque() != &opaque {
                return Err(Error::new(
                    "opaque representation producer changed its requested identity",
                ));
            }
            let mut children = Vec::new();
            collect(record.record.hidden(), &mut children, budget, 0)?;
            edges.push((opaque, children.clone()));
            pending.extend(children);
            records.push(record);
        }
    }
    fn acyclic(
        node: &Arc<ir::OpaqueTy>,
        edges: &[(Arc<ir::OpaqueTy>, Vec<Arc<ir::OpaqueTy>>)],
        path: &mut Vec<Arc<ir::OpaqueTy>>,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        budget.check_depth(path.len()).map_err(Error::new)?;
        budget
            .charge_work(edges.len() + path.len())
            .map_err(Error::new)?;
        if path.contains(node) {
            return Err(Error::new(
                "unsupported: recursive opaque representation dependency",
            ));
        }
        path.push(node.clone());
        if let Some((_, children)) = edges.iter().find(|(owner, _)| owner == node) {
            for child in children {
                acyclic(child, edges, path, budget)?;
            }
        }
        path.pop();
        Ok(())
    }
    for root in &demanded {
        acyclic(root, &edges, &mut Vec::new(), budget)?;
    }
    Ok(Representations {
        records: records.into(),
    })
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
            if !output.contains(opaque) {
                output.push(opaque.clone());
            }
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
        ir::Ty::Closure(value) => {
            for component in value.types() {
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
