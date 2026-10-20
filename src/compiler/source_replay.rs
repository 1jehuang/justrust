//! Private shared source-order inference and canonical source publication.
use super::{
    frontend, ir,
    traits::{self as tr, FactsError},
};
use std::sync::Arc;

pub(super) type InferTerm = Arc<InferKind>;
#[derive(Clone, Debug)]
pub(super) enum InferKind {
    Slot(usize),
    Rigid(ir::Ty),
    Integer,
    Float,
    UnknownFloat,
    Unknown(bool),
    Tuple(Arc<[InferTerm]>),
    Array(InferTerm, usize),
    Slice(InferTerm),
    Ref(ir::BorrowKind, InferTerm),
    RawPtr(ir::BorrowKind, InferTerm),
    Nominal(ir::DeclId, Arc<[InferTerm]>),
    Opaque(Arc<ir::OpaqueTy>, Arc<[InferTerm]>),
}

pub(super) struct CallInferenceState {
    budget: ir::PlanBudget,
    env: Option<Arc<ir::ParamEnv>>,
    slots: Vec<Option<InferTerm>>,
}
impl CallInferenceState {
    pub(super) fn new(
        env: Option<Arc<ir::ParamEnv>>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, FactsError> {
        let count = env.as_ref().map_or(0, |e| e.arity());
        let state = Self {
            budget: budget.clone(),
            env,
            slots: Vec::new(),
        };
        if count != 0 {
            state.storage(count)?;
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| FactsError::resource(budget, "inference slot allocation"))?;
        slots.resize(count, None);
        Ok(Self { slots, ..state })
    }
    pub(super) fn slots(&self) -> &[Option<InferTerm>] {
        &self.slots
    }
    pub(super) fn resolve(
        &self,
        term: &InferTerm,
        fallback: bool,
        depth: usize,
        materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
    ) -> Result<Option<ir::Ty>, FactsError> {
        self.work(depth)?;
        Ok(match term.as_ref() {
            InferKind::Rigid(ty) => Some(ty.clone()),
            InferKind::Opaque(descriptor, terms) => {
                self.storage(terms.len())?;
                let mut actuals = Vec::with_capacity(terms.len());
                for term in terms.iter() {
                    let Some(actual) = self.resolve(term, fallback, depth + 1, materialize)? else {
                        return Ok(None);
                    };
                    actuals.push(actual);
                }
                let mut actuals = actuals.into_iter();
                let mapped = descriptor.try_map(|_| {
                    self.work(depth + 1)?;
                    actuals.next().ok_or_else(|| {
                        FactsError::invalid(&self.budget, "opaque inference lost a child")
                    })
                })?;
                if actuals.next().is_some() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque inference retained extra children",
                    ));
                }
                Some(ir::Ty::Opaque(Arc::new(mapped)))
            }
            InferKind::Integer if fallback => Some(ir::Ty::Int(ir::IntTy::I32)),
            InferKind::Float if fallback => Some(ir::Ty::Float(ir::FloatTy::F64)),
            InferKind::Integer
            | InferKind::Float
            | InferKind::UnknownFloat
            | InferKind::Unknown(_) => None,
            InferKind::Slot(_) => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "unresolved schema slot escaped inference binding",
                ));
            }
            InferKind::Tuple(fields) => {
                self.storage(fields.len())?;
                let mut result = Vec::new();
                result.try_reserve_exact(fields.len()).map_err(|_| {
                    FactsError::resource(&self.budget, "inference tuple allocation")
                })?;
                for field in fields.iter() {
                    let Some(ty) = self.resolve(field, fallback, depth + 1, materialize)? else {
                        return Ok(None);
                    };
                    result.push(ty);
                }
                Some(ir::Ty::Tuple(result.into()))
            }
            InferKind::Ref(kind, term) => {
                let value = self.resolve(term, fallback, depth + 1, materialize)?;
                if let Some(ty) = value {
                    self.storage(1)?;
                    Some(ir::Ty::Ref {
                        kind: *kind,
                        referent: Arc::new(ty),
                    })
                } else {
                    None
                }
            }
            InferKind::RawPtr(kind, term) => {
                let value = self.resolve(term, fallback, depth + 1, materialize)?;
                if let Some(ty) = value {
                    self.storage(1)?;
                    Some(ir::Ty::RawPtr {
                        kind: *kind,
                        pointee: Arc::new(ty),
                    })
                } else {
                    None
                }
            }
            InferKind::Slice(term) => {
                match self.resolve(term, fallback, depth + 1, materialize)? {
                    Some(element) => {
                        self.storage(1)?;
                        Some(ir::Ty::Slice {
                            element: Arc::new(element),
                        })
                    }
                    None => None,
                }
            }
            InferKind::Array(term, len) => {
                let value = self.resolve(term, fallback, depth + 1, materialize)?;
                if let Some(ty) = value {
                    self.storage(1)?;
                    Some(ir::Ty::Array {
                        element: Arc::new(ty),
                        len: *len,
                    })
                } else {
                    None
                }
            }
            InferKind::Nominal(declaration, terms) => {
                self.storage(terms.len())?;
                let mut actuals = Vec::new();
                actuals.try_reserve_exact(terms.len()).map_err(|_| {
                    FactsError::resource(&self.budget, "inference nominal allocation")
                })?;
                for term in terms.iter() {
                    let Some(ty) = self.resolve(term, fallback, depth + 1, materialize)? else {
                        return Ok(None);
                    };
                    actuals.push(ty);
                }
                Some(materialize(*declaration, &actuals)?)
            }
        })
    }
    pub(super) fn finish(
        &self,
        fallback: bool,
        materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
    ) -> Result<Option<Vec<ir::Ty>>, FactsError> {
        self.storage(self.slots.len())?;
        let mut actuals = Vec::new();
        actuals
            .try_reserve_exact(self.slots.len())
            .map_err(|_| FactsError::resource(&self.budget, "inference actual allocation"))?;
        for slot in &self.slots {
            self.work(0)?;
            let Some(term) = slot else { return Ok(None) };
            let Some(ty) = self.resolve(term, fallback, 0, materialize)? else {
                return Ok(None);
            };
            actuals.push(ty);
        }
        Ok(Some(actuals))
    }

    pub(super) fn schema(&self, ty: &ir::Ty, depth: usize) -> Result<InferTerm, FactsError> {
        let env = self
            .env
            .as_ref()
            .ok_or_else(|| FactsError::invalid(&self.budget, "inference schema has no binder"))?;
        self.type_term(ty, Some(env.binder()), depth)
    }
    pub(super) fn work(&self, depth: usize) -> Result<(), FactsError> {
        self.budget
            .check_depth(depth)
            .map_err(|e| FactsError::resource(&self.budget, &e))?;
        self.budget
            .charge_work(1)
            .map_err(|e| FactsError::resource(&self.budget, &e))
    }
    pub(super) fn storage(&self, count: usize) -> Result<(), FactsError> {
        self.budget
            .charge_work(count)
            .map_err(|e| FactsError::resource(&self.budget, &e))?;
        self.budget
            .charge_nodes(1)
            .map_err(|e| FactsError::resource(&self.budget, &e))?;
        let row = std::mem::size_of::<InferTerm>()
            .checked_add(std::mem::size_of::<ir::Ty>())
            .and_then(|n| n.checked_add(64))
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| FactsError::resource(&self.budget, "inference row storage overflow"))?;
        let bytes = count
            .checked_mul(row)
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| FactsError::resource(&self.budget, "inference allocation overflow"))?;
        self.budget
            .retain_bytes(bytes)
            .map_err(|e| FactsError::resource(&self.budget, &e))
    }
    pub(super) fn node(&self, kind: InferKind) -> Result<InferTerm, FactsError> {
        self.budget
            .charge_nodes(1)
            .map_err(|e| FactsError::resource(&self.budget, &e))?;
        self.work(0)?;
        let bytes = std::mem::size_of::<InferKind>()
            .checked_add(128)
            .ok_or_else(|| FactsError::resource(&self.budget, "inference node storage overflow"))?;
        self.budget
            .retain_bytes(bytes)
            .map_err(|e| FactsError::resource(&self.budget, &e))?;
        Ok(Arc::new(kind))
    }
    fn collect_terms(
        &self,
        count: usize,
        terms: impl ExactSizeIterator<Item = Result<InferTerm, FactsError>>,
    ) -> Result<Arc<[InferTerm]>, FactsError> {
        if terms.len() != count {
            return Err(FactsError::invalid(
                &self.budget,
                "inference term count mismatch",
            ));
        }
        // All callers prepay storage(count), covering the live Vec and Arc slice
        // simultaneously, including the empty Arc allocation.
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| FactsError::resource(&self.budget, "inference term buffer allocation"))?;
        for term in terms {
            values.push(term?);
        }
        Ok(values.into())
    }
    fn taint(&self, term: &InferTerm, depth: usize) -> Result<InferTerm, FactsError> {
        self.work(depth)?;
        let kind = match term.as_ref() {
            InferKind::Integer => InferKind::Unknown(true),
            InferKind::Float | InferKind::UnknownFloat => InferKind::UnknownFloat,
            InferKind::Unknown(numeric) => InferKind::Unknown(*numeric),
            InferKind::Tuple(fields) => {
                self.storage(fields.len())?;
                InferKind::Tuple(self.collect_terms(
                    fields.len(),
                    fields.iter().map(|f| self.taint(f, depth + 1)),
                )?)
            }
            InferKind::Ref(kind, referent) => {
                InferKind::Ref(*kind, self.taint(referent, depth + 1)?)
            }
            InferKind::RawPtr(kind, pointee) => {
                InferKind::RawPtr(*kind, self.taint(pointee, depth + 1)?)
            }
            InferKind::Array(element, len) => {
                InferKind::Array(self.taint(element, depth + 1)?, *len)
            }
            InferKind::Slice(element) => InferKind::Slice(self.taint(element, depth + 1)?),
            InferKind::Nominal(decl, args) => {
                self.storage(args.len())?;
                InferKind::Nominal(
                    *decl,
                    self.collect_terms(args.len(), args.iter().map(|a| self.taint(a, depth + 1)))?,
                )
            }
            InferKind::Opaque(descriptor, args) => {
                self.storage(args.len())?;
                InferKind::Opaque(
                    descriptor.clone(),
                    self.collect_terms(args.len(), args.iter().map(|a| self.taint(a, depth + 1)))?,
                )
            }
            _ => return Ok(term.clone()),
        };
        self.node(kind)
    }
    pub(super) fn merge(
        &self,
        a: &InferTerm,
        b: &InferTerm,
        depth: usize,
    ) -> Result<InferTerm, FactsError> {
        self.work(depth)?;
        if Arc::ptr_eq(a, b) {
            return Ok(a.clone());
        }
        match (a.as_ref(), b.as_ref()) {
            (InferKind::Unknown(n), other) | (other, InferKind::Unknown(n)) => {
                if *n
                    && !matches!(
                        other,
                        InferKind::Integer
                            | InferKind::Unknown(_)
                            | InferKind::Rigid(ir::Ty::Int(_))
                    )
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "generic integer-domain constraint conflicts with noninteger type",
                    ));
                }
                if let InferKind::Unknown(m) = other {
                    return self.node(InferKind::Unknown(*n || *m));
                }
                self.taint(
                    if matches!(a.as_ref(), InferKind::Unknown(_)) {
                        b
                    } else {
                        a
                    },
                    depth + 1,
                )
            }
            (InferKind::UnknownFloat, InferKind::Float | InferKind::UnknownFloat)
            | (InferKind::Float, InferKind::UnknownFloat) => self.node(InferKind::UnknownFloat),
            (InferKind::UnknownFloat, InferKind::Rigid(ir::Ty::Float(_))) => Ok(b.clone()),
            (InferKind::Rigid(ir::Ty::Float(_)), InferKind::UnknownFloat) => Ok(a.clone()),
            (InferKind::Float, InferKind::Float) => Ok(a.clone()),
            (InferKind::Float, InferKind::Rigid(ir::Ty::Float(_))) => Ok(b.clone()),
            (InferKind::Rigid(ir::Ty::Float(_)), InferKind::Float) => Ok(a.clone()),
            (InferKind::Integer, InferKind::Integer) => Ok(a.clone()),
            (InferKind::Integer, InferKind::Rigid(ir::Ty::Int(_))) => Ok(b.clone()),
            (InferKind::Rigid(ir::Ty::Int(_)), InferKind::Integer) => Ok(a.clone()),
            (InferKind::Rigid(x), InferKind::Rigid(y))
                if frontend::replay_type_equal(x, y, &self.budget)? =>
            {
                Ok(a.clone())
            }
            (InferKind::Tuple(x), InferKind::Tuple(y)) if x.len() == y.len() => {
                self.storage(x.len())?;
                self.node(InferKind::Tuple(
                    self.collect_terms(
                        x.len(),
                        x.iter()
                            .zip(y.iter())
                            .map(|(x, y)| self.merge(x, y, depth + 1)),
                    )?,
                ))
            }
            (InferKind::Ref(k, x), InferKind::Ref(l, y)) if k == l => {
                self.node(InferKind::Ref(*k, self.merge(x, y, depth + 1)?))
            }
            (InferKind::RawPtr(k, x), InferKind::RawPtr(l, y)) if k == l => {
                self.node(InferKind::RawPtr(*k, self.merge(x, y, depth + 1)?))
            }
            (InferKind::Array(x, n), InferKind::Array(y, m)) if n == m => {
                self.node(InferKind::Array(self.merge(x, y, depth + 1)?, *n))
            }
            (InferKind::Slice(x), InferKind::Slice(y)) => {
                self.node(InferKind::Slice(self.merge(x, y, depth + 1)?))
            }
            (InferKind::Nominal(d, x), InferKind::Nominal(e, y))
                if d == e && x.len() == y.len() =>
            {
                self.storage(x.len())?;
                self.node(InferKind::Nominal(
                    *d,
                    self.collect_terms(
                        x.len(),
                        x.iter()
                            .zip(y.iter())
                            .map(|(x, y)| self.merge(x, y, depth + 1)),
                    )?,
                ))
            }
            (InferKind::Opaque(a, x), InferKind::Opaque(b, y))
                if x.len() == y.len() && opaque_shape_equal(a, b, &self.budget, depth + 1)? =>
            {
                self.storage(x.len())?;
                self.node(InferKind::Opaque(
                    a.clone(),
                    self.collect_terms(
                        x.len(),
                        x.iter()
                            .zip(y.iter())
                            .map(|(x, y)| self.merge(x, y, depth + 1)),
                    )?,
                ))
            }
            _ => Err(FactsError::invalid(
                &self.budget,
                "conflicting generic inference constraints",
            )),
        }
    }
    fn occurs(&self, slot: usize, term: &InferTerm, depth: usize) -> Result<(), FactsError> {
        self.work(depth)?;
        match term.as_ref() {
            InferKind::Slot(index) if *index == slot => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "cyclic generic inference constraint",
                ));
            }
            InferKind::Tuple(fields)
            | InferKind::Nominal(_, fields)
            | InferKind::Opaque(_, fields) => {
                for f in fields.iter() {
                    self.occurs(slot, f, depth + 1)?;
                }
            }
            InferKind::Array(element, _)
            | InferKind::Slice(element)
            | InferKind::Ref(_, element)
            | InferKind::RawPtr(_, element) => self.occurs(slot, element, depth + 1)?,
            _ => {}
        }
        Ok(())
    }
    pub(super) fn constrain(
        &mut self,
        schema: &InferTerm,
        evidence: &InferTerm,
        depth: usize,
    ) -> Result<(), FactsError> {
        self.work(depth)?;
        if matches!(evidence.as_ref(), InferKind::Rigid(ir::Ty::Never))
            || matches!(schema.as_ref(), InferKind::Rigid(ir::Ty::Never))
        {
            return Ok(());
        }
        match schema.as_ref() {
            InferKind::Slot(slot) => {
                self.occurs(*slot, evidence, depth + 1)?;
                let slot_value = self.slots.get(*slot).ok_or_else(|| {
                    FactsError::invalid(&self.budget, "inference slot outside original binder")
                })?;
                let value = if let Some(old) = slot_value {
                    self.merge(old, evidence, depth + 1)?
                } else {
                    evidence.clone()
                };
                self.slots[*slot] = Some(value);
                Ok(())
            }
            InferKind::Tuple(fields) => match evidence.as_ref() {
                InferKind::Tuple(actual) if fields.len() == actual.len() => {
                    for (f, a) in fields.iter().zip(actual.iter()) {
                        self.constrain(f, a, depth + 1)?;
                    }
                    Ok(())
                }
                InferKind::Unknown(_) => {
                    for f in fields.iter() {
                        self.constrain(f, evidence, depth + 1)?;
                    }
                    Ok(())
                }
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic tuple inference shape mismatch",
                )),
            },
            InferKind::Ref(kind, referent) => match evidence.as_ref() {
                InferKind::Ref(actual_kind, actual) if kind == actual_kind => {
                    self.constrain(referent, actual, depth + 1)
                }
                InferKind::Unknown(_) => self.constrain(referent, evidence, depth + 1),
                InferKind::Rigid(ir::Ty::Ref {
                    kind: actual_kind,
                    referent: actual,
                }) if kind == actual_kind => {
                    let actual = InferTerm::new(InferKind::Rigid(actual.as_ref().clone()));
                    self.constrain(referent, &actual, depth + 1)
                }
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic reference inference kind or shape mismatch",
                )),
            },
            InferKind::RawPtr(kind, pointee) => match evidence.as_ref() {
                InferKind::RawPtr(actual_kind, actual) if kind == actual_kind => {
                    self.constrain(pointee, actual, depth + 1)
                }
                InferKind::Unknown(_) => self.constrain(pointee, evidence, depth + 1),
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic raw pointer inference kind or shape mismatch",
                )),
            },
            InferKind::Array(element, len) => match evidence.as_ref() {
                InferKind::Array(actual, n) if len == n => {
                    self.constrain(element, actual, depth + 1)
                }
                InferKind::Unknown(_) => self.constrain(element, evidence, depth + 1),
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic array inference shape mismatch",
                )),
            },
            InferKind::Nominal(decl, args) => match evidence.as_ref() {
                InferKind::Nominal(actual, values)
                    if decl == actual && args.len() == values.len() =>
                {
                    for (f, a) in args.iter().zip(values.iter()) {
                        self.constrain(f, a, depth + 1)?;
                    }
                    Ok(())
                }
                InferKind::Unknown(_) => {
                    for f in args.iter() {
                        self.constrain(f, evidence, depth + 1)?;
                    }
                    Ok(())
                }
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic nominal inference identity mismatch",
                )),
            },
            InferKind::Opaque(descriptor, args) => match evidence.as_ref() {
                InferKind::Opaque(actual, values)
                    if args.len() == values.len()
                        && opaque_shape_equal(descriptor, actual, &self.budget, depth + 1)? =>
                {
                    for (formal, actual) in args.iter().zip(values.iter()) {
                        self.constrain(formal, actual, depth + 1)?;
                    }
                    Ok(())
                }
                InferKind::Unknown(_) => {
                    for formal in args.iter() {
                        self.constrain(formal, evidence, depth + 1)?;
                    }
                    Ok(())
                }
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic opaque inference identity mismatch",
                )),
            },
            InferKind::Slice(element) => match evidence.as_ref() {
                InferKind::Slice(actual) => self.constrain(element, actual, depth + 1),
                InferKind::Unknown(_) => Ok(()),
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic slice inference shape mismatch",
                )),
            },
            InferKind::Rigid(_) => {
                if matches!(evidence.as_ref(), InferKind::Unknown(_)) {
                    Ok(())
                } else {
                    self.merge(schema, evidence, depth + 1).map(|_| ())
                }
            }
            _ => Err(FactsError::invalid(
                &self.budget,
                "invalid generic schema inference marker",
            )),
        }
    }
    pub(super) fn type_term(
        &self,
        ty: &ir::Ty,
        schema: Option<ir::BinderId>,
        depth: usize,
    ) -> Result<InferTerm, FactsError> {
        self.work(depth)?;
        let kind = match ty {
            ir::Ty::Opaque(descriptor) => {
                let count = descriptor.types().count();
                self.storage(count)?;
                let mut children = Vec::new();
                children.try_reserve_exact(count).map_err(|_| {
                    FactsError::resource(&self.budget, "opaque inference child allocation")
                })?;
                for ty in descriptor.types() {
                    children.push(self.type_term(ty, schema, depth + 1)?);
                }
                InferKind::Opaque(descriptor.clone(), children.into())
            }
            ir::Ty::Param(param) if schema == Some(param.owner) => {
                InferKind::Slot(param.index as usize)
            }
            ir::Ty::Tuple(fields) => {
                self.storage(fields.len())?;
                InferKind::Tuple(self.collect_terms(
                    fields.len(),
                    fields.iter().map(|t| self.type_term(t, schema, depth + 1)),
                )?)
            }
            ir::Ty::Ref { kind, referent } => {
                InferKind::Ref(*kind, self.type_term(referent, schema, depth + 1)?)
            }
            ir::Ty::RawPtr { kind, pointee } => {
                InferKind::RawPtr(*kind, self.type_term(pointee, schema, depth + 1)?)
            }
            ir::Ty::Array { element, len } => {
                InferKind::Array(self.type_term(element, schema, depth + 1)?, *len)
            }
            ir::Ty::Slice { element } => {
                InferKind::Slice(self.type_term(element, schema, depth + 1)?)
            }
            ir::Ty::Struct(ty) => {
                self.storage(ty.def().type_args.len())?;
                InferKind::Nominal(
                    ty.def().source_decl,
                    self.collect_terms(
                        ty.def().type_args.len(),
                        ty.def()
                            .type_args
                            .iter()
                            .map(|t| self.type_term(t, schema, depth + 1)),
                    )?,
                )
            }
            ir::Ty::Enum(ty) => {
                self.storage(ty.def().type_args.len())?;
                InferKind::Nominal(
                    ty.def().source_decl,
                    self.collect_terms(
                        ty.def().type_args.len(),
                        ty.def()
                            .type_args
                            .iter()
                            .map(|t| self.type_term(t, schema, depth + 1)),
                    )?,
                )
            }
            _ => InferKind::Rigid(ty.clone()),
        };
        self.node(kind)
    }
}

// The token is retained independently of the enclosing body. No bare address is persisted.
#[derive(Clone, Debug)]
pub(super) struct OriginalBodyId(Arc<OriginalIdentity>);
#[derive(Debug)]
struct OriginalIdentity {
    _private: u8,
}
impl PartialEq for OriginalBodyId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for OriginalBodyId {}
impl OriginalBodyId {
    pub(super) fn new(budget: &ir::PlanBudget) -> Result<Self, FactsError> {
        budget
            .charge_nodes(1)
            .map_err(|e| FactsError::resource(budget, &e))?;
        work(budget, 1)?;
        retain(
            budget,
            std::mem::size_of::<OriginalIdentity>() + 2 * std::mem::size_of::<usize>(),
        )?;
        Ok(Self(Arc::new(OriginalIdentity { _private: 0 })))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceNode {
    pub(super) owner: OriginalBodyId,
    pub(super) path: Arc<[u32]>,
}
impl SourceNode {
    pub(super) fn child(
        &self,
        role: u32,
        index: usize,
        budget: &ir::PlanBudget,
    ) -> Result<Self, FactsError> {
        let len = self
            .path
            .len()
            .checked_add(2)
            .ok_or_else(|| FactsError::resource(budget, "source path work overflow"))?;
        depth(budget, len / 2)?;
        let copies = len
            .checked_mul(2)
            .ok_or_else(|| FactsError::resource(budget, "source path copy work overflow"))?;
        work(budget, copies)?;
        budget
            .charge_nodes(1)
            .map_err(|e| FactsError::resource(budget, &e))?;
        let index = u32::try_from(index)
            .map_err(|_| FactsError::resource(budget, "source path index overflow"))?;
        let row = 2usize
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| FactsError::resource(budget, "source path row overflow"))?;
        retain(
            budget,
            len.checked_mul(row)
                .and_then(|n| n.checked_add(32))
                .ok_or_else(|| FactsError::resource(budget, "source path storage overflow"))?,
        )?;
        let mut path = Vec::new();
        path.try_reserve_exact(len)
            .map_err(|_| FactsError::resource(budget, "source path allocation"))?;
        path.extend_from_slice(&self.path);
        path.push(role);
        path.push(index);
        Ok(Self {
            owner: self.owner.clone(),
            path: path.into(),
        })
    }
}
pub(super) fn work(budget: &ir::PlanBudget, n: usize) -> Result<(), FactsError> {
    budget
        .charge_work(n)
        .map_err(|e| FactsError::resource(budget, &e))
}
pub(super) fn retain(budget: &ir::PlanBudget, n: usize) -> Result<(), FactsError> {
    budget
        .retain_bytes(n)
        .map_err(|e| FactsError::resource(budget, &e))
}
pub(super) fn depth(budget: &ir::PlanBudget, n: usize) -> Result<(), FactsError> {
    budget
        .check_depth(n)
        .map_err(|e| FactsError::resource(budget, &e))
}
pub(super) fn push<T>(
    items: &mut Vec<T>,
    value: T,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    work(budget, 1)?;
    budget
        .charge_nodes(1)
        .map_err(|e| FactsError::resource(budget, &e))?;
    let needed = items
        .len()
        .checked_add(1)
        .ok_or_else(|| FactsError::resource(budget, "source row count overflow"))?;
    if items.len() == items.capacity() {
        let capacity = items.capacity();
        let replacement = if capacity == 0 {
            4
        } else {
            capacity
                .checked_mul(2)
                .ok_or_else(|| FactsError::resource(budget, "source row capacity overflow"))?
                .max(4)
        }
        .max(needed);
        let old = capacity
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| FactsError::resource(budget, "source replay old storage overflow"))?;
        let next = replacement
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| {
                FactsError::resource(budget, "source replay replacement storage overflow")
            })?;
        let bytes = old
            .checked_add(next)
            .and_then(|n| n.checked_add(32))
            .ok_or_else(|| FactsError::resource(budget, "source replay storage overflow"))?;
        // Charge the actual old buffer and requested replacement concurrently,
        // and account relocation before requesting the larger allocation.
        work(budget, items.len())?;
        retain(budget, bytes)?;
        let additional = replacement
            .checked_sub(items.len())
            .ok_or_else(|| FactsError::resource(budget, "source row reservation underflow"))?;
        items
            .try_reserve_exact(additional)
            .map_err(|_| FactsError::resource(budget, "source replay row allocation"))?;
    }
    items.push(value);
    Ok(())
}

pub(super) struct OriginalBody {
    source: frontend::OriginalSource,
}
impl std::fmt::Debug for OriginalBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalBody").finish_non_exhaustive()
    }
}
impl OriginalBody {
    #[cfg(test)]
    pub(super) fn source_for_test(&self) -> frontend::OriginalSource {
        self.source.clone()
    }
    pub(super) fn from_source(
        cap: &frontend::CanonicalSourceCapability,
        source: frontend::OriginalSource,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<Self>, FactsError> {
        cap.validate_original(&source, budget)?;
        budget
            .charge_nodes(1)
            .map_err(|e| FactsError::resource(budget, &e))?;
        retain(budget, std::mem::size_of::<Self>() + 32)?;
        Ok(Arc::new(Self { source }))
    }
}
#[derive(Clone, Debug)]
pub(super) struct NarrowValidatedCall {
    pub(super) call: ir::ExprId,
    pub(super) source: SourceNode,
    pub(super) trace: Arc<SharedIdentityTrace>,
}
#[derive(Debug)]
pub(super) struct ValidatedOriginalCalls {
    original: Arc<OriginalBody>,
    calls: Arc<[NarrowValidatedCall]>,
}
impl ValidatedOriginalCalls {
    pub(super) fn expected(
        &self,
        call: ir::ExprId,
        argument: usize,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ir::CallExpectation>, FactsError> {
        work(budget, self.calls.len())?;
        for row in self.calls.iter() {
            if row.call == call {
                if argument != 0 {
                    return Err(FactsError::invalid(budget, "shared replay argument index"));
                }
                frontend::validate_original_occurrence(&self.original.source, &row.source, budget)?;
                if !matches!(row.trace.before.as_ref(), InferKind::Slot(0))
                    || !Arc::ptr_eq(&row.trace.evidence, &row.trace.after)
                {
                    return Err(FactsError::invalid(
                        budget,
                        "retained shared frontier changed",
                    ));
                }
                return Ok(Some(ir::CallExpectation::NoFirmReference));
            }
        }
        Ok(None)
    }
}
pub(super) struct ReplayedOriginalCalls {
    source: Arc<ir::CheckedFunction>,
    facts: Arc<tr::ValidatedFacts>,
    validated: Arc<ValidatedOriginalCalls>,
}
impl ReplayedOriginalCalls {
    pub(super) fn validate_binding(
        &self,
        source: &Arc<ir::CheckedFunction>,
        facts: &Arc<tr::ValidatedFacts>,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        work(budget, 1)?;
        if !Arc::ptr_eq(source, &self.source)
            || !Arc::ptr_eq(facts, &self.facts)
            || !facts.budget().same_authority(budget)
        {
            return Err(FactsError::invalid(
                budget,
                "replay changed exact source or facts",
            ));
        }
        frontend::validate_original_context(&self.validated.original.source, facts, budget)
    }
    pub(super) fn validated(&self) -> &Arc<ValidatedOriginalCalls> {
        &self.validated
    }
}
pub(super) fn replay_original_calls(
    source: Arc<ir::CheckedFunction>,
    original: &Arc<OriginalBody>,
    facts: &Arc<tr::ValidatedFacts>,
    budget: &ir::PlanBudget,
) -> Result<ReplayedOriginalCalls, FactsError> {
    frontend::validate_original_context(&original.source, facts, budget)?;
    let calls = frontend::replay_source_body(&original.source, &source, facts, budget)?;
    budget
        .charge_nodes(2)
        .map_err(|e| FactsError::resource(budget, &e))?;
    work(budget, calls.len())?;
    retain(
        budget,
        calls
            .len()
            .checked_mul(std::mem::size_of::<NarrowValidatedCall>())
            .and_then(|n| n.checked_add(std::mem::size_of::<ValidatedOriginalCalls>() + 64))
            .ok_or_else(|| FactsError::resource(budget, "replay sealing storage overflow"))?,
    )?;
    let validated = Arc::new(ValidatedOriginalCalls {
        original: original.clone(),
        calls: calls.into(),
    });
    Ok(ReplayedOriginalCalls {
        source,
        facts: facts.clone(),
        validated,
    })
}
#[derive(Debug)]
pub(super) struct CanonicalSourcePublication {
    identity: OriginalBodyId,
    source: Arc<ir::CheckedFunction>,
    original: Arc<OriginalBody>,
    replay: Arc<ValidatedOriginalCalls>,
    facts: Arc<tr::ValidatedFacts>,
}
impl CanonicalSourcePublication {
    /// Authored recipe identity is not the finalized publication token.
    pub(super) fn authored_identity(&self) -> &OriginalBodyId {
        self.original.source.authored_identity()
    }
    pub(super) fn identity(&self) -> &OriginalBodyId {
        &self.identity
    }
    pub(super) fn source(&self) -> &Arc<ir::CheckedFunction> {
        &self.source
    }
    pub(super) fn facts(&self) -> &Arc<tr::ValidatedFacts> {
        &self.facts
    }
    #[cfg(test)]
    pub(super) fn original(&self) -> &Arc<OriginalBody> {
        &self.original
    }
    pub(super) fn replay(&self) -> &Arc<ValidatedOriginalCalls> {
        &self.replay
    }
    pub(super) fn validate_source(
        &self,
        source: &Arc<ir::CheckedFunction>,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        work(budget, 1)?;
        if !Arc::ptr_eq(source, &self.source)
            || !std::ptr::eq(facts, self.facts.as_ref())
            || !facts.budget().same_authority(budget)
        {
            return Err(FactsError::invalid(
                budget,
                "publication changed exact source or facts",
            ));
        }
        source
            .runtime_sites
            .validate(source.exprs.len())
            .map_err(|message| FactsError::invalid(budget, &message))?;
        // The private factory replayed this exact immutable final Arc and performed the
        // mandatory candidate structural validation before mint. No retained payload
        // has interior mutability, and Arc::make_mut edits cannot share this Arc.
        if !Arc::ptr_eq(&self.replay.original, &self.original) {
            return Err(FactsError::invalid(
                budget,
                "publication changed retained replay owner",
            ));
        }
        frontend::validate_original_context(&self.original.source, facts, budget)
    }
}
pub(super) fn publish_canonical_source(
    cap: frontend::CanonicalSourceCapability,
    final_source: Arc<ir::CheckedFunction>,
    original: Arc<OriginalBody>,
    facts: Arc<tr::ValidatedFacts>,
    budget: &ir::PlanBudget,
) -> Result<Arc<CanonicalSourcePublication>, FactsError> {
    cap.validate_original(&original.source, budget)?;
    cap.validate_runtime_sites(&final_source)?;
    let replayed = replay_original_calls(final_source.clone(), &original, &facts, budget)?;
    ir::validate_source_candidate(&final_source, &facts, &replayed, budget)?;
    budget
        .charge_nodes(1)
        .map_err(|e| FactsError::resource(budget, &e))?;
    retain(
        budget,
        std::mem::size_of::<CanonicalSourcePublication>() + 32,
    )?;
    Ok(Arc::new(CanonicalSourcePublication {
        identity: OriginalBodyId::new(budget)?,
        source: final_source,
        original,
        replay: replayed.validated,
        facts,
    }))
}

// The only retained result context in this bounded grammar is source-derived Absent.
#[derive(Clone, Debug)]
pub(super) struct SharedIdentityTrace {
    binder: ir::BinderId,
    before: InferTerm,
    evidence: InferTerm,
    after: InferTerm,
    actual: ir::Ty,
}
fn opaque_shape_equal(
    left: &ir::OpaqueTy,
    right: &ir::OpaqueTy,
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<bool, FactsError> {
    depth(budget, at)?;
    for _ in left
        .captured_regions()
        .iter()
        .chain(right.captured_regions())
    {
        work(budget, 1)?;
    }
    for bound in left.bounds().iter().chain(right.bounds()) {
        work(budget, 1 + bound.args().len() + bound.equalities().len())?;
    }
    left.try_eq(right, |_, _| {
        work(budget, 1)?;
        Ok(true)
    })
}
fn terms_equal(
    left: &InferTerm,
    right: &InferTerm,
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<bool, FactsError> {
    depth(budget, at)?;
    work(budget, 1)?;
    Ok(match (left.as_ref(), right.as_ref()) {
        (InferKind::Slot(a), InferKind::Slot(b)) => a == b,
        (InferKind::Rigid(a), InferKind::Rigid(b)) => frontend::replay_type_equal(a, b, budget)?,
        (InferKind::Integer, InferKind::Integer)
        | (InferKind::Float, InferKind::Float)
        | (InferKind::UnknownFloat, InferKind::UnknownFloat) => true,
        (InferKind::Unknown(a), InferKind::Unknown(b)) => a == b,
        (InferKind::Ref(a, x), InferKind::Ref(b, y))
        | (InferKind::RawPtr(a, x), InferKind::RawPtr(b, y)) => {
            a == b && terms_equal(x, y, budget, at + 1)?
        }
        (InferKind::Array(x, a), InferKind::Array(y, b)) => {
            a == b && terms_equal(x, y, budget, at + 1)?
        }
        (InferKind::Slice(x), InferKind::Slice(y)) => terms_equal(x, y, budget, at + 1)?,
        (InferKind::Opaque(a, x), InferKind::Opaque(b, y)) => {
            if x.len() != y.len() || !opaque_shape_equal(a, b, budget, at + 1)? {
                return Ok(false);
            }
            for (a, b) in x.iter().zip(y.iter()) {
                if !terms_equal(a, b, budget, at + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        (InferKind::Tuple(x), InferKind::Tuple(y))
        | (InferKind::Nominal(_, x), InferKind::Nominal(_, y)) => {
            if let (InferKind::Nominal(a, _), InferKind::Nominal(b, _)) =
                (left.as_ref(), right.as_ref())
                && a != b
            {
                return Ok(false);
            }
            if x.len() != y.len() {
                return Ok(false);
            }
            for (a, b) in x.iter().zip(y.iter()) {
                if !terms_equal(a, b, budget, at + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        _ => false,
    })
}
impl SharedIdentityTrace {
    pub(super) fn actual(&self) -> &ir::Ty {
        &self.actual
    }
    pub(super) fn validate_replayed(
        &self,
        other: &Self,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        work(budget, 4)?;
        if self.binder != other.binder
            || !matches!(self.before.as_ref(), InferKind::Slot(0))
            || !matches!(other.before.as_ref(), InferKind::Slot(0))
            || !Arc::ptr_eq(&self.evidence, &self.after)
            || !Arc::ptr_eq(&other.evidence, &other.after)
        {
            return Err(FactsError::invalid(
                budget,
                "shared replay frontier identity changed",
            ));
        }
        // The common finisher materializes the exact post-commit evidence. Its type comparison is metered.
        if !terms_equal(&self.evidence, &other.evidence, budget, 0)?
            || !frontend::replay_type_equal(&self.actual, &other.actual, budget)?
        {
            return Err(FactsError::invalid(
                budget,
                "shared replay frontier actual changed",
            ));
        }
        Ok(())
    }
}
/// One transition program is called during source checking and immutable replay.
/// Nominal materialization remains in the authoritative caller, never in the term solver.
pub(super) fn infer_shared_identity(
    view: &tr::OriginalCallableView,
    referent: &ir::Ty,
    budget: &ir::PlanBudget,
    materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
) -> Result<Arc<SharedIdentityTrace>, FactsError> {
    infer_reference_identity(view, ir::BorrowKind::Shared, referent, budget, materialize)
}

/// The original source boundary must prove the immediate borrow and its capability.
/// The retained evidence includes the exact kind, so mutable and shared traces
/// cannot validate against each other. This does not widen general inference.
pub(super) fn infer_reference_identity(
    view: &tr::OriginalCallableView,
    kind: ir::BorrowKind,
    referent: &ir::Ty,
    budget: &ir::PlanBudget,
    materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
) -> Result<Arc<SharedIdentityTrace>, FactsError> {
    let env = view.generic_env();
    if env.scopes().len() != 1 || env.scopes()[0].arity() != 1 || view.signature().params.len() != 1
    {
        return Err(FactsError::unsupported(
            budget,
            "shared inference requires exactly one own slot",
        ));
    }
    let env = env.scopes()[0].clone();
    let binder = env.binder();
    let parameter = ir::Ty::Param(ir::ParamId {
        owner: binder,
        index: 0,
    });
    work(budget, 3)?;
    if view.signature().params[0] != parameter || view.signature().result != parameter {
        return Err(FactsError::unsupported(
            budget,
            "shared inference requires original (T)->T",
        ));
    }
    let mut state = CallInferenceState::new(Some(env), budget)?;
    let before = state.schema(&parameter, 0)?;
    // The original None result context seeds no constraint. No defaulting is performed.
    if !matches!(before.as_ref(), InferKind::Slot(0)) || state.slots().iter().any(Option::is_some) {
        return Err(FactsError::invalid(
            budget,
            "nonempty initial shared frontier",
        ));
    }
    state.storage(1)?;
    let input = ir::Ty::Ref {
        kind,
        referent: Arc::new(referent.clone()),
    };
    let evidence = state.type_term(&input, None, 0)?;
    state.constrain(&before, &evidence, 0)?;
    let Some(Some(after)) = state.slots().first() else {
        return Err(FactsError::invalid(budget, "missing final shared frontier"));
    };
    let after = after.clone();
    if !Arc::ptr_eq(&after, &evidence) {
        return Err(FactsError::invalid(
            budget,
            "shared frontier changed evidence",
        ));
    }
    let Some(mut actuals) = state.finish(false, materialize)? else {
        return Err(FactsError::invalid(
            budget,
            "shared final frontier unresolved",
        ));
    };
    if actuals.len() != 1 {
        return Err(FactsError::invalid(
            budget,
            "shared final actual count changed",
        ));
    }
    let actual = actuals.pop().unwrap();
    budget
        .charge_nodes(1)
        .map_err(|e| FactsError::resource(budget, &e))?;
    retain(budget, std::mem::size_of::<SharedIdentityTrace>() + 32)?;
    Ok(Arc::new(SharedIdentityTrace {
        binder,
        before,
        evidence,
        after,
        actual,
    }))
}

#[cfg(test)]
mod publication_identity_tests {
    use super::*;

    fn publish(
        plan: &super::super::frontend::PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Arc<CanonicalSourcePublication> {
        let declaration = plan.callable_decl(0).unwrap();
        let super::super::frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, budget).unwrap()
        else {
            panic!("fixture requires a canonical source publication")
        };
        publication
    }

    #[test]
    fn publication_seals_are_fresh_without_retaining_authority() {
        let budget = ir::PlanBudget::default();
        let program = super::super::frontend::parse("fn main() {}").unwrap();
        let plan = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let first = publish(&plan, &budget);
        let same_body = publish(&plan, &budget);
        assert_eq!(first.identity(), &first.identity().clone());
        assert_eq!(first.facts().declaration(), same_body.facts().declaration());
        assert_ne!(first.identity(), same_body.identity());
        let retained = first.identity().clone();
        drop(first);
        assert_eq!(retained, retained.clone());
        let foreign = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let foreign = publish(&foreign, &budget);
        assert_ne!(&retained, foreign.identity());
        assert_ne!(same_body.identity(), foreign.identity());
        assert_ne!(
            same_body.facts().declaration(),
            foreign.facts().declaration()
        );
    }
}

#[cfg(test)]
mod reference_identity_trace_tests {
    use super::*;

    #[test]
    fn reference_identity_trace_preserves_exact_capability_and_evidence() {
        let budget = ir::PlanBudget::default();
        let binder = ir::BinderId {
            declaration: ir::DeclId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
        };
        let state = CallInferenceState::new(None, &budget).unwrap();
        let trace = |kind| {
            let actual = ir::Ty::Ref {
                kind,
                referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
            };
            let evidence = state.type_term(&actual, None, 0).unwrap();
            SharedIdentityTrace {
                binder,
                before: state.node(InferKind::Slot(0)).unwrap(),
                after: evidence.clone(),
                evidence,
                actual,
            }
        };
        let shared = trace(ir::BorrowKind::Shared);
        let mutable = trace(ir::BorrowKind::Mut);
        shared.validate_replayed(&shared, &budget).unwrap();
        mutable.validate_replayed(&mutable, &budget).unwrap();
        assert!(shared.validate_replayed(&mutable, &budget).is_err());
        assert!(mutable.validate_replayed(&shared, &budget).is_err());

        let mut changed = mutable.clone();
        changed.actual = shared.actual.clone();
        assert!(mutable.validate_replayed(&changed, &budget).is_err());
        let mut replaced = mutable.clone();
        replaced.after = trace(ir::BorrowKind::Mut).after;
        assert!(mutable.validate_replayed(&replaced, &budget).is_err());
    }
}

#[cfg(test)]
mod float_inference_tests {
    use super::*;

    #[test]
    fn char_rigid_inference_never_accepts_numeric_or_boolean_domains() {
        let budget = ir::PlanBudget::default();
        let state = CallInferenceState::new(None, &budget).unwrap();
        let character = state.type_term(&ir::Ty::Char, None, 0).unwrap();
        let mut materialize = |_, _: &[ir::Ty]| unreachable!();
        assert_eq!(
            state
                .resolve(&character, false, 0, &mut materialize)
                .unwrap(),
            Some(ir::Ty::Char)
        );
        for other in [
            InferKind::Integer,
            InferKind::Float,
            InferKind::UnknownFloat,
            InferKind::Rigid(ir::Ty::Int(ir::IntTy::U32)),
            InferKind::Rigid(ir::Ty::Bool),
        ] {
            let other = state.node(other).unwrap();
            assert!(state.merge(&character, &other, 0).is_err());
            assert!(state.merge(&other, &character, 0).is_err());
        }
        let same = state.type_term(&ir::Ty::Char, None, 0).unwrap();
        assert!(state.merge(&character, &same, 0).is_ok());
    }

    #[test]
    fn float_fallback_is_deferred_and_never_integer() {
        let budget = ir::PlanBudget::default();
        let state = CallInferenceState::new(None, &budget).unwrap();
        let float = state.node(InferKind::Float).unwrap();
        let mut materialize = |_, _: &[ir::Ty]| unreachable!();
        assert!(
            state
                .resolve(&float, false, 0, &mut materialize)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            state.resolve(&float, true, 0, &mut materialize).unwrap(),
            Some(ir::Ty::Float(ir::FloatTy::F64))
        );
        for ty in [ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool] {
            let rigid = state.node(InferKind::Rigid(ty)).unwrap();
            assert!(state.merge(&float, &rigid, 0).is_err());
            assert!(state.merge(&rigid, &float, 0).is_err());
        }
        let concrete = state
            .node(InferKind::Rigid(ir::Ty::Float(ir::FloatTy::F32)))
            .unwrap();
        let merged = state.merge(&float, &concrete, 0).unwrap();
        assert_eq!(
            state.resolve(&merged, true, 0, &mut materialize).unwrap(),
            Some(ir::Ty::Float(ir::FloatTy::F32))
        );
    }

    #[test]
    fn unknown_float_evidence_never_forces_fallback_or_crosses_domains() {
        let budget = ir::PlanBudget::default();
        let state = CallInferenceState::new(None, &budget).unwrap();
        let float = state.node(InferKind::Float).unwrap();
        let unknown = state.node(InferKind::Unknown(false)).unwrap();
        let integer = state.node(InferKind::Integer).unwrap();
        let mut materialize = |_, _: &[ir::Ty]| unreachable!();
        for (a, b) in [(&float, &unknown), (&unknown, &float)] {
            let tainted = state.merge(a, b, 0).unwrap();
            assert!(
                state
                    .resolve(&tainted, true, 0, &mut materialize)
                    .unwrap()
                    .is_none()
            );
            assert!(state.merge(&tainted, &integer, 0).is_err());
            assert!(state.merge(&integer, &tainted, 0).is_err());
            let concrete = state
                .node(InferKind::Rigid(ir::Ty::Float(ir::FloatTy::F32)))
                .unwrap();
            let merged = state.merge(&tainted, &concrete, 0).unwrap();
            assert_eq!(
                state.resolve(&merged, true, 0, &mut materialize).unwrap(),
                Some(ir::Ty::Float(ir::FloatTy::F32))
            );
        }
    }
}
