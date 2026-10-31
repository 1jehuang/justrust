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
    FnPointer(Arc<[InferTerm]>, InferTerm),
    PhantomData(ir::DeclId, InferTerm),
    Cell(ir::DeclId, InferTerm),
    ManuallyDrop(ir::DeclId, InferTerm),
    Array(InferTerm, usize),
    Slice(InferTerm),
    Ref(ir::BorrowKind, InferTerm),
    RawPtr(ir::BorrowKind, InferTerm),
    Nominal(ir::DeclId, Arc<[InferTerm]>),
    Opaque(Arc<ir::OpaqueTy>, Arc<[InferTerm]>),
    Projection(ir::DeclId, ir::DeclId, Arc<[InferTerm]>),
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
            InferKind::Cell(declaration, argument) => {
                let Some(argument) = self.resolve(argument, fallback, depth + 1, materialize)?
                else {
                    return Ok(None);
                };
                let ty = materialize(*declaration, std::slice::from_ref(&argument))?;
                let ir::Ty::Cell(cell) = &ty else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "Cell inference materialized a foreign type",
                    ));
                };
                if cell.declaration() != *declaration
                    || !replay_types_equal_at(cell.argument(), &argument, &self.budget, depth + 1)?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "Cell inference changed original owner or actual",
                    ));
                }
                Some(ty)
            }
            InferKind::ManuallyDrop(declaration, argument) => {
                let Some(argument) = self.resolve(argument, fallback, depth + 1, materialize)?
                else {
                    return Ok(None);
                };
                let ty = materialize(*declaration, std::slice::from_ref(&argument))?;
                let ir::Ty::ManuallyDrop(manual_wrapper) = &ty else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "manual_wrapper inference materialized a non-manual_wrapper type",
                    ));
                };
                if manual_wrapper.declaration() != *declaration
                    || !replay_types_equal_at(
                        manual_wrapper.argument(),
                        &argument,
                        &self.budget,
                        depth + 1,
                    )?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "manual_wrapper inference changed original owner or actual",
                    ));
                }
                Some(ty)
            }
            InferKind::PhantomData(declaration, argument) => {
                let Some(argument) = self.resolve(argument, fallback, depth + 1, materialize)?
                else {
                    return Ok(None);
                };
                let ty = materialize(*declaration, std::slice::from_ref(&argument))?;
                let ir::Ty::PhantomData(marker) = &ty else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "marker inference materialized a non-marker type",
                    ));
                };
                if marker.declaration() != *declaration
                    || !replay_types_equal_at(
                        marker.argument(),
                        &argument,
                        &self.budget,
                        depth + 1,
                    )?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "marker inference changed original owner or actual",
                    ));
                }
                Some(ty)
            }
            InferKind::Projection(declaration, item, terms) => {
                self.storage(terms.len())?;
                let mut actuals = Vec::new();
                actuals.try_reserve_exact(terms.len()).map_err(|_| {
                    FactsError::resource(&self.budget, "projection inference allocation")
                })?;
                for term in terms.iter() {
                    let Some(ty) = self.resolve(term, fallback, depth + 1, materialize)? else {
                        return Ok(None);
                    };
                    actuals.push(ty);
                }
                let mut actuals = actuals.into_iter();
                let self_ty = actuals.next().ok_or_else(|| {
                    FactsError::invalid(&self.budget, "projection inference lost receiver")
                })?;
                Some(ir::Ty::AssociatedType(Arc::new(
                    ir::AssociatedTypeProjection {
                        trait_ref: ir::TraitRef {
                            declaration: *declaration,
                            self_ty,
                            args: actuals.collect(),
                        },
                        item: *item,
                    },
                )))
            }
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
            InferKind::FnPointer(inputs, output) => {
                let mut types = ir::FnPointerTy::storage(inputs.len(), &self.budget)
                    .map_err(|e| FactsError::from_message(&self.budget, &e))?;
                for t in inputs.iter() {
                    let Some(ty) = self.resolve(t, fallback, depth + 1, materialize)? else {
                        return Ok(None);
                    };
                    types.push(ty);
                }
                let Some(output) = self.resolve(output, fallback, depth + 1, materialize)? else {
                    return Ok(None);
                };
                Some(ir::Ty::FnPointer(
                    ir::FnPointerTy::new(types, output, &self.budget)
                        .map_err(|e| FactsError::from_message(&self.budget, &e))?,
                ))
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
    /// Instantiate only explicit own-slot markers. Rigid caller parameters
    /// are leaves and must never be rebound by index or display spelling.
    fn bind_schema(
        &self,
        schema: &InferTerm,
        slots: &[InferTerm],
        depth: usize,
    ) -> Result<InferTerm, FactsError> {
        self.work(depth)?;
        let kind = match schema.as_ref() {
            InferKind::Cell(owner, argument) => {
                InferKind::Cell(*owner, self.bind_schema(argument, slots, depth + 1)?)
            }
            InferKind::ManuallyDrop(owner, argument) => {
                InferKind::ManuallyDrop(*owner, self.bind_schema(argument, slots, depth + 1)?)
            }
            InferKind::PhantomData(owner, argument) => {
                InferKind::PhantomData(*owner, self.bind_schema(argument, slots, depth + 1)?)
            }
            InferKind::Slot(index) => {
                let bound = slots.get(*index).ok_or_else(|| {
                    FactsError::invalid(&self.budget, "result schema slot outside inferred binder")
                })?;
                // A rigid actual still contributes its full type depth at
                // this insertion point. Do not hide a deep result behind a
                // constant-cost clone of the already materialized value.
                if let InferKind::Rigid(ty) = bound.as_ref()
                    && !replay_types_equal_at(ty, ty, &self.budget, depth)?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "invalid rigid result actual",
                    ));
                }
                return Ok(bound.clone());
            }
            InferKind::FnPointer(inputs, output) => InferKind::FnPointer(
                self.collect_pointer_terms(
                    inputs.len(),
                    inputs.iter().map(|t| self.bind_schema(t, slots, depth + 1)),
                )?,
                self.bind_schema(output, slots, depth + 1)?,
            ),
            InferKind::Tuple(fields) => {
                self.storage(fields.len())?;
                InferKind::Tuple(
                    self.collect_terms(
                        fields.len(),
                        fields
                            .iter()
                            .map(|term| self.bind_schema(term, slots, depth + 1)),
                    )?,
                )
            }
            InferKind::Nominal(declaration, fields) => {
                self.storage(fields.len())?;
                InferKind::Nominal(
                    *declaration,
                    self.collect_terms(
                        fields.len(),
                        fields
                            .iter()
                            .map(|term| self.bind_schema(term, slots, depth + 1)),
                    )?,
                )
            }
            InferKind::Opaque(descriptor, fields) => {
                self.storage(fields.len())?;
                InferKind::Opaque(
                    descriptor.clone(),
                    self.collect_terms(
                        fields.len(),
                        fields
                            .iter()
                            .map(|term| self.bind_schema(term, slots, depth + 1)),
                    )?,
                )
            }
            InferKind::Projection(declaration, item, fields) => {
                self.storage(fields.len())?;
                InferKind::Projection(
                    *declaration,
                    *item,
                    self.collect_terms(
                        fields.len(),
                        fields
                            .iter()
                            .map(|term| self.bind_schema(term, slots, depth + 1)),
                    )?,
                )
            }
            InferKind::Ref(kind, inner) => {
                InferKind::Ref(*kind, self.bind_schema(inner, slots, depth + 1)?)
            }
            InferKind::RawPtr(kind, inner) => {
                InferKind::RawPtr(*kind, self.bind_schema(inner, slots, depth + 1)?)
            }
            InferKind::Array(inner, len) => {
                InferKind::Array(self.bind_schema(inner, slots, depth + 1)?, *len)
            }
            InferKind::Slice(inner) => {
                InferKind::Slice(self.bind_schema(inner, slots, depth + 1)?)
            }
            _ => return Ok(schema.clone()),
        };
        self.node(kind)
    }
    /// Add one independently authenticated input type to the call's constraints.
    /// The caller owns source authentication. This operation neither consults
    /// checked result types nor applies fallback to unresolved inputs.
    /// Return the immutable constraint terms for callers retaining a trace.
    pub(super) fn constrain_input(
        &mut self,
        parameter: &ir::Ty,
        input: &ir::Ty,
    ) -> Result<(InferTerm, InferTerm), FactsError> {
        let schema = self.schema(parameter, 0)?;
        let evidence = self.type_term(input, None, 0)?;
        self.constrain(&schema, &evidence, 0)?;
        Ok((schema, evidence))
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
    fn collect_pointer_terms(
        &self,
        count: usize,
        terms: impl ExactSizeIterator<Item = Result<InferTerm, FactsError>>,
    ) -> Result<Arc<[InferTerm]>, FactsError> {
        let error = |e: String| FactsError::from_message(&self.budget, &e);
        let mut values = ir::FnPointerTy::storage(count, &self.budget).map_err(error)?;
        if terms.len() != count {
            return Err(FactsError::invalid(
                &self.budget,
                "function pointer inference count mismatch",
            ));
        }
        for term in terms {
            values.push(term?);
        }
        ir::FnPointerTy::slice(values, &self.budget).map_err(error)
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
            InferKind::Cell(owner, argument) => {
                InferKind::Cell(*owner, self.taint(argument, depth + 1)?)
            }
            InferKind::ManuallyDrop(owner, argument) => {
                InferKind::ManuallyDrop(*owner, self.taint(argument, depth + 1)?)
            }
            InferKind::PhantomData(owner, argument) => {
                InferKind::PhantomData(*owner, self.taint(argument, depth + 1)?)
            }
            InferKind::Integer => InferKind::Unknown(true),
            InferKind::Float | InferKind::UnknownFloat => InferKind::UnknownFloat,
            InferKind::Unknown(numeric) => InferKind::Unknown(*numeric),
            InferKind::FnPointer(inputs, output) => InferKind::FnPointer(
                self.collect_pointer_terms(
                    inputs.len(),
                    inputs.iter().map(|t| self.taint(t, depth + 1)),
                )?,
                self.taint(output, depth + 1)?,
            ),
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
            InferKind::Projection(declaration, item, args) => {
                self.storage(args.len())?;
                InferKind::Projection(
                    *declaration,
                    *item,
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
            (InferKind::Cell(a, x), InferKind::Cell(b, y)) if a == b => {
                self.node(InferKind::Cell(*a, self.merge(x, y, depth + 1)?))
            }
            (InferKind::ManuallyDrop(a, x), InferKind::ManuallyDrop(b, y)) if a == b => {
                self.node(InferKind::ManuallyDrop(*a, self.merge(x, y, depth + 1)?))
            }
            (InferKind::PhantomData(a, x), InferKind::PhantomData(b, y)) if a == b => {
                self.node(InferKind::PhantomData(*a, self.merge(x, y, depth + 1)?))
            }
            (InferKind::FnPointer(x, xo), InferKind::FnPointer(y, yo)) if x.len() == y.len() => {
                self.node(InferKind::FnPointer(
                    self.collect_pointer_terms(
                        x.len(),
                        x.iter()
                            .zip(y.iter())
                            .map(|(x, y)| self.merge(x, y, depth + 1)),
                    )?,
                    self.merge(xo, yo, depth + 1)?,
                ))
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
            (InferKind::Projection(d, i, x), InferKind::Projection(e, j, y))
                if d == e && i == j && x.len() == y.len() =>
            {
                self.storage(x.len())?;
                self.node(InferKind::Projection(
                    *d,
                    *i,
                    self.collect_terms(
                        x.len(),
                        x.iter()
                            .zip(y.iter())
                            .map(|(x, y)| self.merge(x, y, depth + 1)),
                    )?,
                ))
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
                &format!("conflicting generic inference constraints: {a:?} versus {b:?}"),
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
            InferKind::FnPointer(inputs, output) => {
                for t in inputs.iter().chain(std::iter::once(output)) {
                    self.occurs(slot, t, depth + 1)?;
                }
            }
            InferKind::Tuple(fields)
            | InferKind::Nominal(_, fields)
            | InferKind::Projection(_, _, fields)
            | InferKind::Opaque(_, fields) => {
                for f in fields.iter() {
                    self.occurs(slot, f, depth + 1)?;
                }
            }
            InferKind::Cell(_, element)
            | InferKind::ManuallyDrop(_, element)
            | InferKind::PhantomData(_, element)
            | InferKind::Array(element, _)
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
            InferKind::Cell(owner, argument) => match evidence.as_ref() {
                InferKind::Cell(actual_owner, actual) if owner == actual_owner => {
                    self.constrain(argument, actual, depth + 1)
                }
                InferKind::Unknown(_) => self.constrain(argument, evidence, depth + 1),
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "Cell inference owner or actual mismatch",
                )),
            },
            InferKind::ManuallyDrop(owner, argument) => match evidence.as_ref() {
                InferKind::ManuallyDrop(actual_owner, actual) if owner == actual_owner => {
                    self.constrain(argument, actual, depth + 1)
                }
                InferKind::Unknown(_) => self.constrain(argument, evidence, depth + 1),
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic manual_wrapper inference owner or shape mismatch",
                )),
            },
            InferKind::PhantomData(owner, argument) => match evidence.as_ref() {
                InferKind::PhantomData(actual_owner, actual) if owner == actual_owner => {
                    self.constrain(argument, actual, depth + 1)
                }
                InferKind::Unknown(_) => self.constrain(argument, evidence, depth + 1),
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic marker inference owner or shape mismatch",
                )),
            },
            InferKind::FnPointer(inputs, output) => match evidence.as_ref() {
                InferKind::FnPointer(actual, result) if inputs.len() == actual.len() => {
                    for (t, a) in inputs.iter().zip(actual.iter()) {
                        self.constrain(t, a, depth + 1)?;
                    }
                    self.constrain(output, result, depth + 1)
                }
                InferKind::Unknown(_) => {
                    for t in inputs.iter().chain(std::iter::once(output)) {
                        self.constrain(t, evidence, depth + 1)?;
                    }
                    Ok(())
                }
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "generic function pointer inference shape mismatch",
                )),
            },
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
            // Projection families are not injective. Do not infer their
            // arguments from an expected output or structurally invert them.
            InferKind::Projection(_, _, _) | InferKind::Rigid(_) => {
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
            ir::Ty::AssociatedType(projection) => {
                let count = projection
                    .trait_ref
                    .args
                    .len()
                    .checked_add(1)
                    .ok_or_else(|| {
                        FactsError::resource(&self.budget, "projection inference child overflow")
                    })?;
                self.storage(count)?;
                let mut children = Vec::new();
                children.try_reserve_exact(count).map_err(|_| {
                    FactsError::resource(&self.budget, "projection inference child allocation")
                })?;
                for ty in std::iter::once(&projection.trait_ref.self_ty)
                    .chain(projection.trait_ref.args.iter())
                {
                    children.push(self.type_term(ty, schema, depth + 1)?);
                }
                InferKind::Projection(
                    projection.trait_ref.declaration,
                    projection.item,
                    children.into(),
                )
            }
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
                if self.env.as_ref().is_some_and(|env| !env.contains(*param)) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "inference schema parameter outside original binder",
                    ));
                }
                InferKind::Slot(param.index as usize)
            }
            ir::Ty::Cell(cell) => InferKind::Cell(
                cell.declaration(),
                self.type_term(cell.argument(), schema, depth + 1)?,
            ),
            ir::Ty::ManuallyDrop(wrapper) => InferKind::ManuallyDrop(
                wrapper.declaration(),
                self.type_term(wrapper.argument(), schema, depth + 1)?,
            ),
            ir::Ty::PhantomData(marker) => InferKind::PhantomData(
                marker.declaration(),
                self.type_term(marker.argument(), schema, depth + 1)?,
            ),
            ir::Ty::FnPointer(pointer) => InferKind::FnPointer(
                self.collect_pointer_terms(
                    pointer.inputs.len(),
                    pointer
                        .inputs
                        .iter()
                        .map(|t| self.type_term(t, schema, depth + 1)),
                )?,
                self.type_term(&pointer.output, schema, depth + 1)?,
            ),
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

/// Source replay bridges canonical concrete source applications to typed graph
/// realizations, whose provenance names the original symbolic template rather
/// than the concrete source slot. This is not full metadata validation:
/// callers must still authenticate source declarations and graph input facts.
/// In particular, neither names nor a coincidentally equal field layout grant
/// nominal equality, and a bare graph coordinate is never source authority.
pub(super) fn replay_types_equal(
    left: &ir::Ty,
    right: &ir::Ty,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    replay_types_equal_at(left, right, budget, 0)
}

fn nominal_source_identity(
    ty: &ir::Ty,
    identity: ir::NominalAppId,
    realization: Option<&super::nominal_realization::ReadNominal>,
    budget: &ir::PlanBudget,
) -> Result<(ir::NominalAppId, Option<ir::NominalAppId>), FactsError> {
    work(budget, 1)?;
    if let Some(realization) = realization {
        realization
            .check_definition(ty)
            .map_err(|error| FactsError::invalid(budget, &error.to_string()))?;
        // This is the declaration TEMPLATE application, not the concrete
        // source interner slot for the realized ordered actuals.
        Ok((realization.source_application(), Some(identity)))
    } else if identity.index.is_graph() {
        Err(FactsError::invalid(
            budget,
            "source equality requires a typed nominal realization",
        ))
    } else {
        Ok((identity, None))
    }
}

fn nominal_replay_identity_equal(
    left: (ir::NominalAppId, Option<ir::NominalAppId>),
    right: (ir::NominalAppId, Option<ir::NominalAppId>),
    declaration: ir::DeclId,
    left_args: &[ir::Ty],
    right_args: &[ir::Ty],
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    work(budget, 1)?;
    match (left.1, right.1) {
        // Neither source nor graph coordinates may be weakened within their
        // own domain. In particular, equal source provenance does not authorize
        // a graph output from a foreign engine (or another node/output).
        (None, None) => {
            if left.0 == right.0 {
                return Ok(true);
            }
            // Two source slots for one application can differ only in how
            // nested nominal actuals were identified (source slot versus
            // typed graph realization). Authenticate each slot against its
            // own ordered actuals. The caller has already proved those
            // actuals replay-equal, so both name the same application.
            Ok(left.0.snapshot == declaration.snapshot
                && right.0.snapshot == declaration.snapshot
                && left.0 == frontend::replay_nominal_source_id(declaration, left_args, budget)?
                && right.0 == frontend::replay_nominal_source_id(declaration, right_args, budget)?)
        }
        (Some(a), Some(b)) => Ok(a == b),
        _ => {
            let (source, source_args, template) = if left.1.is_some() {
                (right.0, right_args, left.0)
            } else {
                (left.0, left_args, right.0)
            };
            if source.snapshot != declaration.snapshot || template.snapshot != declaration.snapshot
            {
                return Ok(false);
            }
            // The typed read authenticates (declaration, concrete actuals).
            // Its retained source_application names the symbolic template, so
            // compare the source side with its concrete canonical identity,
            // not with that template's different symbolic-argument digest.
            Ok(source == frontend::replay_nominal_source_id(declaration, source_args, budget)?)
        }
    }
}

fn replay_type_lists_equal(
    left: &[ir::Ty],
    right: &[ir::Ty],
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<bool, FactsError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        if !replay_types_equal_at(left, right, budget, at)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn replay_types_equal_at(
    left: &ir::Ty,
    right: &ir::Ty,
    budget: &ir::PlanBudget,
    at: usize,
) -> Result<bool, FactsError> {
    depth(budget, at)?;
    work(budget, 1)?;
    match (left, right) {
        (ir::Ty::Struct(a), ir::Ty::Struct(b)) => {
            let a_id = nominal_source_identity(left, a.id(), a.realization(), budget)?;
            let b_id = nominal_source_identity(right, b.id(), b.realization(), budget)?;
            Ok(a.def().source_decl == b.def().source_decl
                && replay_type_lists_equal(&a.def().type_args, &b.def().type_args, budget, at + 1)?
                && nominal_replay_identity_equal(
                    a_id,
                    b_id,
                    a.def().source_decl,
                    &a.def().type_args,
                    &b.def().type_args,
                    budget,
                )?)
        }
        (ir::Ty::Enum(a), ir::Ty::Enum(b)) => {
            let a_id = nominal_source_identity(left, a.id(), a.realization(), budget)?;
            let b_id = nominal_source_identity(right, b.id(), b.realization(), budget)?;
            Ok(a.def().source_decl == b.def().source_decl
                && replay_type_lists_equal(&a.def().type_args, &b.def().type_args, budget, at + 1)?
                && nominal_replay_identity_equal(
                    a_id,
                    b_id,
                    a.def().source_decl,
                    &a.def().type_args,
                    &b.def().type_args,
                    budget,
                )?)
        }
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) => replay_type_lists_equal(a, b, budget, at + 1),
        (
            ir::Ty::Ref {
                kind: a,
                referent: x,
            },
            ir::Ty::Ref {
                kind: b,
                referent: y,
            },
        )
        | (
            ir::Ty::RawPtr {
                kind: a,
                pointee: x,
            },
            ir::Ty::RawPtr {
                kind: b,
                pointee: y,
            },
        ) => Ok(a == b && replay_types_equal_at(x, y, budget, at + 1)?),
        (ir::Ty::Array { element: a, len: n }, ir::Ty::Array { element: b, len: m }) => {
            Ok(n == m && replay_types_equal_at(a, b, budget, at + 1)?)
        }
        (ir::Ty::Slice { element: a }, ir::Ty::Slice { element: b }) => {
            replay_types_equal_at(a, b, budget, at + 1)
        }
        (ir::Ty::Cell(a), ir::Ty::Cell(b)) => Ok(a.declaration() == b.declaration()
            && replay_types_equal_at(a.argument(), b.argument(), budget, at + 1)?),
        (ir::Ty::ManuallyDrop(a), ir::Ty::ManuallyDrop(b)) => Ok(a.declaration()
            == b.declaration()
            && replay_types_equal_at(a.argument(), b.argument(), budget, at + 1)?),
        (ir::Ty::PhantomData(a), ir::Ty::PhantomData(b)) => Ok(a.declaration() == b.declaration()
            && replay_types_equal_at(a.argument(), b.argument(), budget, at + 1)?),
        (ir::Ty::FnPointer(a), ir::Ty::FnPointer(b)) => {
            a.try_eq(b, |a, b| replay_types_equal_at(a, b, budget, at + 1))
        }
        (ir::Ty::FnItem(a), ir::Ty::FnItem(b)) => Ok(a.declaration == b.declaration
            && replay_type_lists_equal(&a.type_args, &b.type_args, budget, at + 1)?),
        (ir::Ty::AssociatedType(a), ir::Ty::AssociatedType(b)) => Ok(a.item == b.item
            && a.trait_ref.declaration == b.trait_ref.declaration
            && replay_types_equal_at(&a.trait_ref.self_ty, &b.trait_ref.self_ty, budget, at + 1)?
            && replay_type_lists_equal(&a.trait_ref.args, &b.trait_ref.args, budget, at + 1)?),
        (ir::Ty::Dyn(a), ir::Ty::Dyn(b)) => {
            if a.principal != b.principal
                || a.associated_types.len() != b.associated_types.len()
                || !replay_type_lists_equal(&a.args, &b.args, budget, at + 1)?
            {
                return Ok(false);
            }
            for ((a_item, a), (b_item, b)) in
                a.associated_types.iter().zip(b.associated_types.iter())
            {
                work(budget, 1)?;
                if a_item != b_item || !replay_types_equal_at(a, b, budget, at + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::Opaque(a), ir::Ty::Opaque(b)) => {
            if !opaque_shape_equal(a, b, budget, at + 1)? {
                return Ok(false);
            }
            a.try_eq(b, |a, b| replay_types_equal_at(a, b, budget, at + 1))
        }
        (ir::Ty::Closure(a), ir::Ty::Closure(b)) => {
            for source in a.sources().iter().chain(b.sources()) {
                work(budget, 1)?;
                work(budget, source.place.projection.len())?;
            }
            a.try_eq(b, |a, b| replay_types_equal_at(a, b, budget, at + 1))
        }
        // Every recursive representation is handled above. This fallback only
        // compares rigid leaves or immediately unequal variant discriminants.
        _ => Ok(left == right),
    }
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
pub(super) enum CallTrace {
    Identity(Arc<SharedIdentityTrace>),
    Inputs(Arc<CallInputTrace>),
}
impl From<Arc<SharedIdentityTrace>> for CallTrace {
    fn from(trace: Arc<SharedIdentityTrace>) -> Self {
        Self::Identity(trace)
    }
}
impl From<Arc<CallInputTrace>> for CallTrace {
    fn from(trace: Arc<CallInputTrace>) -> Self {
        Self::Inputs(trace)
    }
}
impl CallTrace {
    fn expected(
        &self,
        argument: usize,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CallExpectation, FactsError> {
        match self {
            Self::Identity(trace) => {
                if argument != 0 {
                    return Err(FactsError::invalid(budget, "shared replay argument index"));
                }
                if !matches!(trace.before.as_ref(), InferKind::Slot(0))
                    || !Arc::ptr_eq(&trace.evidence, &trace.after)
                {
                    return Err(FactsError::invalid(
                        budget,
                        "retained shared frontier changed",
                    ));
                }
                Ok(ir::CallExpectation::NoFirmReference)
            }
            Self::Inputs(trace) => trace.expected(argument, budget),
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct NarrowValidatedCall {
    pub(super) call: ir::ExprId,
    pub(super) source: SourceNode,
    pub(super) trace: CallTrace,
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
                frontend::validate_original_occurrence(&self.original.source, &row.source, budget)?;
                return row.trace.expected(argument, budget).map(Some);
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
#[cfg(test)]
pub(super) fn replay_original_calls(
    source: Arc<ir::CheckedFunction>,
    original: &Arc<OriginalBody>,
    facts: &Arc<tr::ValidatedFacts>,
    budget: &ir::PlanBudget,
) -> Result<ReplayedOriginalCalls, FactsError> {
    replay_original_calls_with_scratch(
        source,
        original,
        facts,
        budget,
        &mut tr::TraitScratch::default(),
    )
}
fn replay_original_calls_with_scratch(
    source: Arc<ir::CheckedFunction>,
    original: &Arc<OriginalBody>,
    facts: &Arc<tr::ValidatedFacts>,
    budget: &ir::PlanBudget,
    scratch: &mut tr::TraitScratch,
) -> Result<ReplayedOriginalCalls, FactsError> {
    frontend::validate_original_context(&original.source, facts, budget)?;
    let calls = frontend::replay_source_body_with_scratch(
        &original.source,
        &source,
        facts,
        budget,
        scratch,
    )?;
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
    scratch: &mut tr::TraitScratch,
) -> Result<Arc<CanonicalSourcePublication>, FactsError> {
    cap.validate_original(&original.source, budget)?;
    cap.validate_runtime_sites(&final_source)?;
    let replayed = replay_original_calls_with_scratch(
        final_source.clone(),
        &original,
        &facts,
        budget,
        scratch,
    )?;
    ir::validate_source_candidate(&final_source, &facts, &replayed, budget, scratch)?;
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

/// Immutable replay of constraints from every original source argument. This is
/// not source authentication: callers must independently recover input types
/// from authored expressions, never from the checked call or its actuals.
/// An optional expected result must likewise come from independent authored
/// context. Numeric fallback is never applied by this API.
#[derive(Clone, Debug)]
pub(super) struct CallInputTrace {
    binder: Option<ir::BinderId>,
    schemas: Arc<[InferTerm]>,
    result_schema: InferTerm,
    // Compared only by the test-side replay validator.
    #[cfg_attr(not(test), allow(dead_code))]
    expected: Option<InferTerm>,
    evidence: Arc<[InferTerm]>,
    slots: Arc<[InferTerm]>,
    actuals: Arc<[ir::Ty]>,
}
impl CallInputTrace {
    #[cfg(test)]
    pub(super) fn actuals(&self) -> &[ir::Ty] {
        &self.actuals
    }

    fn validate_shape(&self, budget: &ir::PlanBudget) -> Result<(), FactsError> {
        work(budget, 1)?;
        if self.schemas.len() != self.evidence.len()
            || self.slots.len() != self.actuals.len()
            || (self.binder.is_none() && !self.slots.is_empty())
        {
            return Err(FactsError::invalid(
                budget,
                "source inference trace arity changed",
            ));
        }
        Ok(())
    }

    /// Reconstruct a returned call's type solely from its original result
    /// schema and source-inferred own slots. Materialization is deliberately
    /// lazy: callers that only need actuals do not demand result nominals.
    /// The callback retains responsibility for authenticated nominal inputs.
    pub(super) fn result_type(
        &self,
        budget: &ir::PlanBudget,
        materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
    ) -> Result<ir::Ty, FactsError> {
        self.validate_shape(budget)?;
        let state = CallInferenceState::new(None, budget)?;
        // Finished actuals came from original source constraints. Preserve
        // their exact typed nominal capabilities rather than re-interning a
        // nominal embedded in an input slot during returned-type replay.
        state.storage(self.actuals.len())?;
        let bindings = state.collect_terms(
            self.actuals.len(),
            self.actuals
                .iter()
                .map(|actual| state.node(InferKind::Rigid(actual.clone()))),
        )?;
        let result = state.bind_schema(&self.result_schema, &bindings, 0)?;
        state
            .resolve(&result, false, 0, materialize)?
            .ok_or_else(|| {
                FactsError::invalid(budget, "source-inferred result remained unresolved")
            })
    }

    /// The authored input schema, not the inferred actual, determines whether
    /// checking this argument supplied a firm reference expectation. In
    /// particular, `T = &mut U` must preserve a move where `&mut T` reborrows.
    pub(super) fn expected(
        &self,
        argument: usize,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CallExpectation, FactsError> {
        self.validate_shape(budget)?;
        let schema = self.schemas.get(argument).ok_or_else(|| {
            FactsError::invalid(budget, "input expectation argument outside signature")
        })?;
        Ok(match schema.as_ref() {
            InferKind::Ref(kind, _) => ir::CallExpectation::FirmReference { kind: *kind },
            _ => ir::CallExpectation::NoFirmReference,
        })
    }

    /// Compare ordered inferred actuals, including rigid caller parameters.
    pub(super) fn validate_actuals(
        &self,
        actuals: &[ir::Ty],
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        self.validate_shape(budget)?;
        if actuals.len() != self.actuals.len() {
            return Err(FactsError::invalid(
                budget,
                "source input actual count changed",
            ));
        }
        for (inferred, actual) in self.actuals.iter().zip(actuals) {
            if !replay_types_equal(inferred, actual, budget)? {
                return Err(FactsError::invalid(budget, "source input actual changed"));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    /// `other` must be independently reconstructed from original source.
    pub(super) fn validate_replayed(
        &self,
        other: &Self,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        self.validate_shape(budget)?;
        other.validate_shape(budget)?;
        if self.binder != other.binder
            || self.schemas.len() != other.schemas.len()
            || self.evidence.len() != other.evidence.len()
            || self.slots.len() != other.slots.len()
            || !terms_equal(&self.result_schema, &other.result_schema, budget, 0)?
        {
            return Err(FactsError::invalid(budget, "source input frontier changed"));
        }
        match (&self.expected, &other.expected) {
            (None, None) => {}
            (Some(left), Some(right)) if terms_equal(left, right, budget, 0)? => {}
            _ => {
                return Err(FactsError::invalid(
                    budget,
                    "source expected result changed",
                ));
            }
        }
        for (left, right) in self
            .schemas
            .iter()
            .zip(other.schemas.iter())
            .chain(self.evidence.iter().zip(other.evidence.iter()))
            .chain(self.slots.iter().zip(other.slots.iter()))
        {
            if !terms_equal(left, right, budget, 0)? {
                return Err(FactsError::invalid(
                    budget,
                    "source input constraint changed",
                ));
            }
        }
        self.validate_actuals(&other.actuals, budget)
    }
}

/// Infer all own slots from the full original input schema. Extra enclosing
/// binders are not guessed: until their own-slot boundary is authenticated by
/// the caller, multi-scope callable views remain explicitly unsupported.
pub(super) fn infer_call_input_trace(
    view: &tr::OriginalCallableView,
    inputs: &[ir::Ty],
    budget: &ir::PlanBudget,
    materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
) -> Result<Arc<CallInputTrace>, FactsError> {
    infer_call_input_trace_with_context(view, inputs, None, budget, materialize)
}

/// As above, with an independently authenticated source result annotation.
/// `expected` must never be obtained from the checked call's result or actuals.
/// It seeds constraints before the input evidence without weakening rigidity.
pub(super) fn infer_call_input_trace_with_context(
    view: &tr::OriginalCallableView,
    inputs: &[ir::Ty],
    expected: Option<&ir::Ty>,
    budget: &ir::PlanBudget,
    materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
) -> Result<Arc<CallInputTrace>, FactsError> {
    let scopes = view.generic_env().scopes();
    if scopes.len() > 1 {
        return Err(FactsError::unsupported(
            budget,
            "input inference requires an authenticated own binder",
        ));
    }
    infer_input_schema_trace_with_context(
        scopes.first().cloned(),
        &view.signature().params,
        &view.signature().result,
        inputs,
        expected,
        budget,
        materialize,
    )
}

fn infer_input_schema_trace(
    env: Option<Arc<ir::ParamEnv>>,
    parameters: &[ir::Ty],
    result: &ir::Ty,
    inputs: &[ir::Ty],
    budget: &ir::PlanBudget,
    materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
) -> Result<Arc<CallInputTrace>, FactsError> {
    infer_input_schema_trace_with_context(
        env,
        parameters,
        result,
        inputs,
        None,
        budget,
        materialize,
    )
}

fn infer_input_schema_trace_with_context(
    env: Option<Arc<ir::ParamEnv>>,
    parameters: &[ir::Ty],
    result: &ir::Ty,
    inputs: &[ir::Ty],
    expected: Option<&ir::Ty>,
    budget: &ir::PlanBudget,
    materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
) -> Result<Arc<CallInputTrace>, FactsError> {
    work(budget, 1)?;
    if parameters.len() != inputs.len() {
        return Err(FactsError::invalid(
            budget,
            "source input signature arity changed",
        ));
    }
    let binder = env.as_ref().map(|env| env.binder());
    let mut state = CallInferenceState::new(env, budget)?;
    state.storage(parameters.len())?;
    let mut schemas = Vec::new();
    let mut evidence = Vec::new();
    schemas
        .try_reserve_exact(parameters.len())
        .map_err(|_| FactsError::resource(budget, "input schema trace allocation"))?;
    evidence
        .try_reserve_exact(inputs.len())
        .map_err(|_| FactsError::resource(budget, "input evidence trace allocation"))?;
    let result_schema = state.type_term(result, binder, 0)?;
    let expected = expected
        .map(|ty| state.type_term(ty, None, 0))
        .transpose()?;
    if let Some(expected) = &expected {
        state.constrain(&result_schema, expected, 0)?;
    }
    for (parameter, input) in parameters.iter().zip(inputs) {
        let schema = state.type_term(parameter, binder, 0)?;
        let actual = state.type_term(input, None, 0)?;
        state.constrain(&schema, &actual, 0)?;
        schemas.push(schema);
        evidence.push(actual);
    }
    let actuals = state.finish(false, materialize)?.ok_or_else(|| {
        FactsError::unsupported(budget, "source evidence leaves generic slots unresolved")
    })?;
    state.storage(state.slots.len())?;
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(state.slots.len())
        .map_err(|_| FactsError::resource(budget, "input frontier trace allocation"))?;
    for slot in state.slots() {
        slots.push(
            slot.as_ref()
                .ok_or_else(|| FactsError::invalid(budget, "resolved input slot missing"))?
                .clone(),
        );
    }
    state.storage(1)?;
    Ok(Arc::new(CallInputTrace {
        binder,
        schemas: schemas.into(),
        result_schema,
        expected,
        evidence: evidence.into(),
        slots: slots.into(),
        actuals: actuals.into(),
    }))
}

// Legacy identity wrappers retain their narrower source expectation contract.
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
        (InferKind::Projection(a, i, x), InferKind::Projection(b, j, y)) => {
            if a != b || i != j || x.len() != y.len() {
                return Ok(false);
            }
            for (a, b) in x.iter().zip(y.iter()) {
                if !terms_equal(a, b, budget, at + 1)? {
                    return Ok(false);
                }
            }
            true
        }
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
        (InferKind::Cell(a, x), InferKind::Cell(b, y)) => {
            a == b && terms_equal(x, y, budget, at + 1)?
        }
        (InferKind::ManuallyDrop(a, x), InferKind::ManuallyDrop(b, y)) => {
            a == b && terms_equal(x, y, budget, at + 1)?
        }
        (InferKind::PhantomData(a, x), InferKind::PhantomData(b, y)) => {
            a == b && terms_equal(x, y, budget, at + 1)?
        }
        (InferKind::FnPointer(x, xo), InferKind::FnPointer(y, yo)) => {
            if x.len() != y.len() {
                return Ok(false);
            }
            let mut equal = terms_equal(xo, yo, budget, at + 1)?;
            for (x, y) in x.iter().zip(y.iter()) {
                equal &= terms_equal(x, y, budget, at + 1)?;
            }
            equal
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
/// Reconstruct the one input-owned generic actual from original source type
/// evidence. There is no result context or numeric fallback in this transition.
pub(super) fn infer_single_input_trace(
    view: &tr::OriginalCallableView,
    input: &ir::Ty,
    budget: &ir::PlanBudget,
) -> Result<Arc<SharedIdentityTrace>, FactsError> {
    if !matches!(input, ir::Ty::Ref { referent, .. }
        if matches!(referent.as_ref(), ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char))
    {
        return Err(FactsError::unsupported(
            budget,
            "input inference requires a scalar reference",
        ));
    }
    let scopes = view.generic_env().scopes();
    if scopes.len() != 1 || scopes[0].arity() != 1 {
        return Err(FactsError::unsupported(
            budget,
            "input inference requires one own slot",
        ));
    }
    let parameter = ir::Ty::Param(ir::ParamId {
        owner: scopes[0].binder(),
        index: 0,
    });
    if view.signature().params.first() != Some(&parameter) {
        return Err(FactsError::unsupported(
            budget,
            "input inference requires original first parameter T",
        ));
    }
    let trace = infer_input_schema_trace(
        Some(scopes[0].clone()),
        std::slice::from_ref(&parameter),
        &view.signature().result,
        std::slice::from_ref(input),
        budget,
        &mut |_, _| {
            Err(FactsError::invalid(
                budget,
                "scalar input unexpectedly materialized a nominal",
            ))
        },
    )?;
    legacy_identity_trace(&trace, budget)
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
    // Reuse the all-input solver without weakening this wrapper's authored
    // identity-call expectation contract.
    retain(budget, std::mem::size_of::<ir::Ty>() + 32)?;
    let input = ir::Ty::Ref {
        kind,
        referent: Arc::new(referent.clone()),
    };
    let trace = infer_call_input_trace(view, &[input], budget, materialize)?;
    legacy_identity_trace(&trace, budget)
}

fn legacy_identity_trace(
    trace: &CallInputTrace,
    budget: &ir::PlanBudget,
) -> Result<Arc<SharedIdentityTrace>, FactsError> {
    work(budget, 1)?;
    let Some(binder) = trace.binder else {
        return Err(FactsError::invalid(
            budget,
            "identity trace lost its binder",
        ));
    };
    if trace.schemas.len() != 1
        || trace.evidence.len() != 1
        || trace.slots.len() != 1
        || trace.actuals.len() != 1
        || !matches!(trace.schemas[0].as_ref(), InferKind::Slot(0))
        || !Arc::ptr_eq(&trace.evidence[0], &trace.slots[0])
    {
        return Err(FactsError::invalid(
            budget,
            "identity trace changed its frontier",
        ));
    }
    budget
        .charge_nodes(1)
        .map_err(|e| FactsError::resource(budget, &e))?;
    retain(budget, std::mem::size_of::<SharedIdentityTrace>() + 32)?;
    Ok(Arc::new(SharedIdentityTrace {
        binder,
        before: trace.schemas[0].clone(),
        evidence: trace.evidence[0].clone(),
        after: trace.slots[0].clone(),
        actual: trace.actuals[0].clone(),
    }))
}

#[cfg(test)]
mod replay_type_equality_tests {
    use super::*;

    fn source_form(ty: &ir::Ty, budget: &ir::PlanBudget) -> ir::Ty {
        match ty {
            ir::Ty::Struct(value) => {
                let mut definition = value.def().clone();
                definition.type_args = definition
                    .type_args
                    .iter()
                    .map(|ty| source_form(ty, budget))
                    .collect();
                definition.id = frontend::replay_nominal_source_id(
                    definition.source_decl,
                    &definition.type_args,
                    budget,
                )
                .unwrap();
                ir::Ty::Struct(ir::StructTy::new(definition))
            }
            ir::Ty::Enum(value) => {
                let mut definition = value.def().clone();
                definition.type_args = definition
                    .type_args
                    .iter()
                    .map(|ty| source_form(ty, budget))
                    .collect();
                definition.id = frontend::replay_nominal_source_id(
                    definition.source_decl,
                    &definition.type_args,
                    budget,
                )
                .unwrap();
                ir::Ty::Enum(ir::EnumTy::new(definition))
            }
            ir::Ty::Ref { kind, referent } => ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(source_form(referent, budget)),
            },
            _ => ty.clone(),
        }
    }

    #[test]
    fn source_and_graph_nominals_compare_by_authenticated_original_application() {
        let compiled = super::super::compile(
            "struct Empty;enum Tag{A}struct Pair<T,U>{first:T,second:U}enum Maybe<T>{Some(T),None}fn main(){let x=Empty;let y=Tag::A;let p=Pair::<u32,bool>{first:1u32,second:true};let m=Maybe::<u32>::Some(2u32);}".into(),
            1,
            false,
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let mut count = 0;
        let mut generic_count = 0;
        for graph in compiled
            .control_flow
            .iter()
            .flat_map(|f| f.locals.iter())
            .map(|l| &l.ty)
        {
            let has_capability = match graph {
                ir::Ty::Struct(value) => value.realization().is_some(),
                ir::Ty::Enum(value) => value.realization().is_some(),
                _ => false,
            };
            if !has_capability {
                continue;
            }
            count += 1;
            let source = source_form(graph, &budget);
            match graph {
                ir::Ty::Struct(value) if !value.def().type_args.is_empty() => {
                    let ir::Ty::Struct(source) = &source else {
                        unreachable!()
                    };
                    assert_ne!(
                        source.id(),
                        value.realization().unwrap().source_application()
                    );
                    generic_count += 1;
                }
                ir::Ty::Enum(value) if !value.def().type_args.is_empty() => {
                    let ir::Ty::Enum(source) = &source else {
                        unreachable!()
                    };
                    assert_ne!(
                        source.id(),
                        value.realization().unwrap().source_application()
                    );
                    generic_count += 1;
                }
                _ => {}
            }
            assert_ne!(&source, graph);
            assert!(replay_types_equal(&source, graph, &budget).unwrap());
            assert!(replay_types_equal(graph, &source, &budget).unwrap());
            let own = Arc::new(
                ir::ParamEnv::new(
                    ir::BinderId {
                        declaration: ir::DeclId {
                            snapshot: ir::SnapshotId::standalone().unwrap(),
                            index: 0,
                        },
                    },
                    1,
                )
                .unwrap(),
            );
            let parameter = ir::Ty::Param(ir::ParamId {
                owner: own.binder(),
                index: 0,
            });
            let trace = infer_input_schema_trace(
                Some(own),
                std::slice::from_ref(&parameter),
                &parameter,
                std::slice::from_ref(graph),
                &budget,
                &mut |_, _| Ok(graph.clone()),
            )
            .unwrap();
            // Returning T must preserve the exact finished graph actual, not
            // re-intern it through a caller callback under a different ID.
            let returned = trace
                .result_type(&budget, &mut |_, _| {
                    panic!("finished nominal actual was unnecessarily reconstructed")
                })
                .unwrap();
            assert!(replay_types_equal(&returned, graph, &budget).unwrap());
            let nested = |ty: ir::Ty| ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(ir::Ty::Tuple(Arc::from([
                    ir::Ty::Array {
                        element: Arc::new(ty),
                        len: 2,
                    },
                    ir::Ty::Bool,
                ]))),
            };
            assert!(
                replay_types_equal(&nested(source.clone()), &nested(graph.clone()), &budget)
                    .unwrap()
            );
            let (stripped, forged, foreign, changed_args) = match graph {
                ir::Ty::Struct(value) => {
                    let stripped = ir::Ty::Struct(ir::StructTy::new(value.def().clone()));
                    let forged = ir::Ty::Struct(
                        ir::StructTy::new(value.def().clone())
                            .with_realization(value.realization().unwrap().clone()),
                    );
                    let ir::Ty::Struct(source) = &source else {
                        unreachable!()
                    };
                    let mut foreign = source.def().clone();
                    foreign.id.index = ir::NominalIndex::SourceSlot(
                        foreign.id.index.source_slot().unwrap().wrapping_add(1),
                    );
                    let mut changed_args = source.def().clone();
                    changed_args.type_args = Arc::from([ir::Ty::Bool]);
                    (
                        stripped,
                        forged,
                        ir::Ty::Struct(ir::StructTy::new(foreign)),
                        ir::Ty::Struct(ir::StructTy::new(changed_args)),
                    )
                }
                ir::Ty::Enum(value) => {
                    let stripped = ir::Ty::Enum(ir::EnumTy::new(value.def().clone()));
                    let forged = ir::Ty::Enum(
                        ir::EnumTy::new(value.def().clone())
                            .with_realization(value.realization().unwrap().clone()),
                    );
                    let ir::Ty::Enum(source) = &source else {
                        unreachable!()
                    };
                    let mut foreign = source.def().clone();
                    foreign.id.index = ir::NominalIndex::SourceSlot(
                        foreign.id.index.source_slot().unwrap().wrapping_add(1),
                    );
                    let mut changed_args = source.def().clone();
                    changed_args.type_args = Arc::from([ir::Ty::Bool]);
                    (
                        stripped,
                        forged,
                        ir::Ty::Enum(ir::EnumTy::new(foreign)),
                        ir::Ty::Enum(ir::EnumTy::new(changed_args)),
                    )
                }
                _ => unreachable!(),
            };
            assert!(replay_types_equal(&stripped, graph, &budget).is_err());
            assert!(replay_types_equal(&forged, graph, &budget).is_err());
            assert!(!replay_types_equal(&foreign, graph, &budget).unwrap());
            assert!(!replay_types_equal(&source, &foreign, &budget).unwrap());
            assert!(!replay_types_equal(&changed_args, graph, &budget).unwrap());
        }
        assert!(
            count >= 2,
            "fixture must retain typed graph struct and enum locals"
        );
        assert!(
            generic_count >= 2,
            "fixture must include concrete generic struct and enum applications"
        );
    }

    #[test]
    fn source_type_equality_preserves_rigid_identity_shape_and_budgets() {
        let budget = ir::PlanBudget::default();
        let param = |snapshot| {
            ir::Ty::Param(ir::ParamId {
                owner: ir::BinderId {
                    declaration: ir::DeclId { snapshot, index: 0 },
                },
                index: 0,
            })
        };
        let a = param(ir::SnapshotId::standalone().unwrap());
        let b = param(ir::SnapshotId::standalone().unwrap());
        assert!(replay_types_equal(&a, &a, &budget).unwrap());
        assert!(!replay_types_equal(&a, &b, &budget).unwrap());
        let tuple = ir::Ty::Tuple(Arc::from([a.clone(), b.clone()]));
        let reordered = ir::Ty::Tuple(Arc::from([b, a]));
        assert!(!replay_types_equal(&tuple, &reordered, &budget).unwrap());
        let tiny = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(
            replay_types_equal(&tuple, &tuple, &tiny)
                .unwrap_err()
                .is_resource()
        );
    }
}

#[cfg(test)]
mod call_input_trace_tests {
    use super::*;

    fn env(count: usize) -> Arc<ir::ParamEnv> {
        Arc::new(
            ir::ParamEnv::new(
                ir::BinderId {
                    declaration: ir::DeclId {
                        snapshot: ir::SnapshotId::standalone().unwrap(),
                        index: 0,
                    },
                },
                count,
            )
            .unwrap(),
        )
    }
    fn param(env: &ir::ParamEnv, index: u16) -> ir::Ty {
        ir::Ty::Param(ir::ParamId {
            owner: env.binder(),
            index,
        })
    }
    fn reference(kind: ir::BorrowKind, ty: ir::Ty) -> ir::Ty {
        ir::Ty::Ref {
            kind,
            referent: Arc::new(ty),
        }
    }
    fn array(ty: ir::Ty, len: usize) -> ir::Ty {
        ir::Ty::Array {
            element: Arc::new(ty),
            len,
        }
    }
    fn infer(
        env: Arc<ir::ParamEnv>,
        parameters: &[ir::Ty],
        inputs: &[ir::Ty],
        budget: &ir::PlanBudget,
    ) -> Result<Arc<CallInputTrace>, FactsError> {
        infer_input_schema_trace(
            Some(env),
            parameters,
            &ir::Ty::Unit,
            inputs,
            budget,
            &mut |_, _| panic!("fixture has no nominal materialization"),
        )
    }

    #[test]
    fn recursive_call_result_keeps_same_binder_input_rigid_and_meters_full_depth() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let parameter = param(&own, 0);
        let result = reference(ir::BorrowKind::Shared, parameter.clone());
        // A recursive call's caller parameter has the same declaration ID as
        // the callee slot, but source input evidence is still rigid, not a
        // unification variable or an occurs-check cycle.
        let recursive = infer_input_schema_trace_with_context(
            Some(own.clone()),
            std::slice::from_ref(&parameter),
            &result,
            std::slice::from_ref(&parameter),
            Some(&result),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(recursive.actuals(), std::slice::from_ref(&parameter));
        assert_eq!(
            recursive
                .result_type(&budget, &mut |_, _| unreachable!())
                .unwrap(),
            result
        );

        let deep_actual = reference(
            ir::BorrowKind::Shared,
            reference(ir::BorrowKind::Shared, ir::Ty::Char),
        );
        let deep_result = reference(
            ir::BorrowKind::Shared,
            reference(ir::BorrowKind::Shared, parameter.clone()),
        );
        let deep = infer_input_schema_trace(
            Some(own),
            &[parameter],
            &deep_result,
            &[deep_actual],
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let limited = ir::PlanBudget::new(ir::PlanLimits {
            depth: 3,
            ..Default::default()
        });
        assert!(
            deep.result_type(&limited, &mut |_, _| unreachable!())
                .unwrap_err()
                .is_resource()
        );
    }

    #[test]
    fn associated_result_substitutes_only_source_inferred_own_slots() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let caller = env(1);
        let declaration = own.binder().declaration;
        let item = caller.binder().declaration;
        let projection = |argument| {
            ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                trait_ref: ir::TraitRef {
                    declaration,
                    self_ty: ir::Ty::Bool,
                    args: Arc::from([argument, param(&caller, 0)]),
                },
                item,
            }))
        };
        let result = projection(param(&own, 0));
        let trace = infer_input_schema_trace(
            Some(own.clone()),
            &[param(&own, 0)],
            &result,
            &[ir::Ty::Char],
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(
            trace
                .result_type(&budget, &mut |_, _| unreachable!())
                .unwrap(),
            projection(ir::Ty::Char)
        );
        let limited = ir::PlanBudget::new(ir::PlanLimits {
            depth: 0,
            ..Default::default()
        });
        assert!(
            trace
                .result_type(&limited, &mut |_, _| unreachable!())
                .unwrap_err()
                .is_resource()
        );

        let mut forged = (*trace).clone();
        let InferKind::Projection(owner, _, children) = forged.result_schema.as_ref() else {
            unreachable!()
        };
        forged.result_schema =
            Arc::new(InferKind::Projection(*owner, declaration, children.clone()));
        assert!(trace.validate_replayed(&forged, &budget).is_err());
        let state = CallInferenceState::new(None, &budget).unwrap();
        for case in [
            "declaration",
            "receiver",
            "ordered-own-and-caller",
            "rigid-caller-owner",
            "missing-child",
            "extra-child",
            "invalid-slot",
        ] {
            let InferKind::Projection(owner, member, children) = trace.result_schema.as_ref()
            else {
                unreachable!()
            };
            let mut owner = *owner;
            let mut children = children.to_vec();
            match case {
                "declaration" => owner = item,
                "receiver" => children[0] = state.node(InferKind::Rigid(ir::Ty::Char)).unwrap(),
                "ordered-own-and-caller" => children.swap(1, 2),
                "rigid-caller-owner" => {
                    children[2] = state.node(InferKind::Rigid(param(&own, 0))).unwrap();
                }
                "missing-child" => {
                    children.pop();
                }
                "extra-child" => children.push(state.node(InferKind::Rigid(ir::Ty::Bool)).unwrap()),
                "invalid-slot" => children[1] = state.node(InferKind::Slot(1)).unwrap(),
                _ => unreachable!(),
            }
            let mut forged = (*trace).clone();
            forged.result_schema = state
                .node(InferKind::Projection(owner, *member, children.into()))
                .unwrap();
            assert_eq!(
                trace
                    .validate_replayed(&forged, &budget)
                    .unwrap_err()
                    .kind(),
                tr::FactsErrorKind::InvalidMetadata,
                "{case}",
            );
            if case == "invalid-slot" {
                assert_eq!(
                    forged
                        .result_type(&budget, &mut |_, _| unreachable!())
                        .unwrap_err()
                        .kind(),
                    tr::FactsErrorKind::InvalidMetadata,
                );
            }
        }
        assert!(state.occurs(0, &trace.result_schema, 0).is_err());
    }

    #[test]
    fn returned_reference_chain_uses_only_source_inferred_results() {
        let budget = ir::PlanBudget::default();
        let caller = env(1);
        let rigid = param(&caller, 0);
        let first = env(1);
        let first_result = reference(ir::BorrowKind::Shared, param(&first, 0));
        let first_trace = infer_input_schema_trace(
            Some(first.clone()),
            &[param(&first, 0)],
            &first_result,
            std::slice::from_ref(&rigid),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let first_output = first_trace
            .result_type(&budget, &mut |_, _| unreachable!())
            .unwrap();
        assert_eq!(
            first_output,
            reference(ir::BorrowKind::Shared, rigid.clone())
        );

        let second = env(1);
        let second_result = reference(ir::BorrowKind::Shared, param(&second, 0));
        let annotation = reference(ir::BorrowKind::Shared, first_output.clone());
        let second_trace = infer_input_schema_trace_with_context(
            Some(second.clone()),
            &[param(&second, 0)],
            &second_result,
            std::slice::from_ref(&first_output),
            Some(&annotation),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let replay = infer_input_schema_trace_with_context(
            Some(second.clone()),
            &[param(&second, 0)],
            &second_result,
            std::slice::from_ref(&first_output),
            Some(&annotation),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(
            second_trace
                .result_type(&budget, &mut |_, _| unreachable!())
                .unwrap(),
            annotation
        );
        second_trace.validate_replayed(&replay, &budget).unwrap();

        let state = CallInferenceState::new(None, &budget).unwrap();
        let mut changed = (*replay).clone();
        changed.result_schema = state
            .node(InferKind::Ref(
                ir::BorrowKind::Mut,
                state.node(InferKind::Slot(0)).unwrap(),
            ))
            .unwrap();
        assert!(second_trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.expected = None;
        assert!(second_trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        let foreign = param(&env(1), 0);
        changed.expected = Some(
            state
                .type_term(
                    &reference(
                        ir::BorrowKind::Shared,
                        reference(ir::BorrowKind::Shared, foreign),
                    ),
                    None,
                    0,
                )
                .unwrap(),
        );
        assert!(second_trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.actuals = Arc::from([reference(ir::BorrowKind::Shared, ir::Ty::Bool)]);
        assert!(second_trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.slots = Arc::from([state.type_term(&ir::Ty::Bool, None, 0).unwrap()]);
        assert!(second_trace.validate_replayed(&changed, &budget).is_err());
        let tiny = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(
            second_trace
                .result_type(&tiny, &mut |_, _| unreachable!())
                .unwrap_err()
                .is_resource()
        );
    }

    #[test]
    fn zero_input_nested_result_rejects_forged_context_schema_and_actuals() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let caller = env(1);
        let nested = |ty| {
            reference(
                ir::BorrowKind::Shared,
                ir::Ty::Tuple(Arc::from([
                    reference(ir::BorrowKind::Shared, ty),
                    ir::Ty::Bool,
                ])),
            )
        };
        let result = nested(param(&own, 0));
        let expected = nested(param(&caller, 0));
        let trace = infer_input_schema_trace_with_context(
            Some(own.clone()),
            &[],
            &result,
            &[],
            Some(&expected),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(
            trace
                .result_type(&budget, &mut |_, _| unreachable!())
                .unwrap(),
            expected
        );
        assert!(
            infer_input_schema_trace(
                Some(own.clone()),
                &[],
                &result,
                &[],
                &budget,
                &mut |_, _| unreachable!()
            )
            .is_err()
        );
        let replay = infer_input_schema_trace_with_context(
            Some(own),
            &[],
            &result,
            &[],
            Some(&expected),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        trace.validate_replayed(&replay, &budget).unwrap();
        let state = CallInferenceState::new(None, &budget).unwrap();
        let mut changed = (*replay).clone();
        changed.result_schema = state.node(InferKind::Slot(9)).unwrap();
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        assert!(
            changed
                .result_type(&budget, &mut |_, _| unreachable!())
                .is_err()
        );
        let mut changed = (*replay).clone();
        changed.expected = Some(state.type_term(&nested(ir::Ty::Char), None, 0).unwrap());
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.actuals = Arc::from([ir::Ty::Char]);
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.slots = Arc::from([]);
        assert!(
            changed
                .result_type(&budget, &mut |_, _| unreachable!())
                .is_err()
        );
        assert!(trace.validate_replayed(&changed, &budget).is_err());
    }

    #[test]
    fn expected_result_infers_zero_input_nominal_output() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let expansion = super::super::expansion_budget::ExpansionBudget::default();
        let modules = super::super::module_identity::ModuleArenaBuilder::for_catalog(
            expansion.clone(),
            super::super::owner_path::standalone_issued(
                &expansion,
                super::super::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        let declaration = env(0).binder().declaration;
        let option = |argument: ir::Ty, index| {
            ir::Ty::Enum(ir::EnumTy::new(ir::EnumDef {
                id: ir::NominalAppId {
                    snapshot: declaration.snapshot,
                    index: ir::NominalIndex::SourceSlot(index),
                },
                source_decl: declaration,
                type_args: Arc::from([argument.clone()]),
                name: "Option".into(),
                module: modules.core_root(),
                visibility: ir::Visibility::Public,
                variants: Arc::from([
                    ir::VariantDef {
                        name: "None".into(),
                        shape: ir::VariantShape::Unit,
                        fields: [].into(),
                    },
                    ir::VariantDef {
                        name: "Some".into(),
                        shape: ir::VariantShape::Tuple,
                        fields: [ir::FieldDef {
                            name: "0".into(),
                            ty: argument,
                            visibility: ir::Visibility::Public,
                            lifetime_bound: false,
                        }]
                        .into(),
                    },
                ]),
            }))
        };
        let result = option(param(&own, 0), 0);
        let inferred = reference(ir::BorrowKind::Shared, ir::Ty::Int(ir::IntTy::I32));
        let expected = option(inferred.clone(), 1);
        let trace = infer_input_schema_trace_with_context(
            Some(own.clone()),
            &[],
            &result,
            &[],
            Some(&expected),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(trace.actuals(), &[inferred]);
        let reconstructed = trace
            .result_type(&budget, &mut |actual_declaration, actuals| {
                assert_eq!(actual_declaration, declaration);
                assert_eq!(actuals.len(), 1);
                Ok(option(actuals[0].clone(), 1))
            })
            .unwrap();
        assert!(replay_types_equal(&reconstructed, &expected, &budget).unwrap());
        let replay = infer_input_schema_trace_with_context(
            Some(own.clone()),
            &[],
            &result,
            &[],
            Some(&expected),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        trace.validate_replayed(&replay, &budget).unwrap();
        assert!(
            infer_input_schema_trace(
                Some(own),
                &[],
                &result,
                &[],
                &budget,
                &mut |_, _| unreachable!()
            )
            .is_err()
        );
        assert!(trace.expected(0, &budget).is_err());
    }

    #[test]
    fn expected_result_conflicts_and_context_mutations_are_rejected() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let parameter = param(&own, 0);
        let parameters = [parameter.clone()];
        let inputs = [ir::Ty::Char];
        let solve = |expected: Option<&ir::Ty>| {
            infer_input_schema_trace_with_context(
                Some(own.clone()),
                &parameters,
                &parameter,
                &inputs,
                expected,
                &budget,
                &mut |_, _| unreachable!(),
            )
        };
        let trace = solve(Some(&ir::Ty::Char)).unwrap();
        assert!(solve(Some(&ir::Ty::Bool)).is_err());
        let absent = solve(None).unwrap();
        assert_eq!(trace.actuals(), absent.actuals());
        assert!(trace.validate_replayed(&absent, &budget).is_err());
        assert!(absent.validate_replayed(&trace, &budget).is_err());
        let mut changed = (*trace).clone();
        changed.expected = Some(Arc::new(InferKind::Rigid(ir::Ty::Bool)));
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*trace).clone();
        changed.expected = None;
        assert!(trace.validate_replayed(&changed, &budget).is_err());
    }

    #[test]
    fn expected_result_keeps_caller_parameters_rigid() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let caller = env(1);
        let result = reference(ir::BorrowKind::Shared, param(&own, 0));
        let expected = reference(ir::BorrowKind::Shared, param(&caller, 0));
        let trace = infer_input_schema_trace_with_context(
            Some(own.clone()),
            &[],
            &result,
            &[],
            Some(&expected),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(trace.actuals(), &[param(&caller, 0)]);
        assert!(
            infer_input_schema_trace_with_context(
                Some(own.clone()),
                &[param(&own, 0)],
                &result,
                &[ir::Ty::Char],
                Some(&expected),
                &budget,
                &mut |_, _| unreachable!()
            )
            .is_err()
        );
        let foreign = env(1);
        let foreign_expected = reference(ir::BorrowKind::Shared, param(&foreign, 0));
        let foreign_trace = infer_input_schema_trace_with_context(
            Some(own),
            &[],
            &result,
            &[],
            Some(&foreign_expected),
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert!(trace.validate_replayed(&foreign_trace, &budget).is_err());
    }

    #[test]
    fn input_expectations_follow_source_schema_not_reference_actuals() {
        let budget = ir::PlanBudget::default();
        let own = env(3);
        let parameters = [
            param(&own, 0),
            reference(ir::BorrowKind::Mut, param(&own, 1)),
            reference(ir::BorrowKind::Shared, param(&own, 2)),
        ];
        let inputs = [
            reference(ir::BorrowKind::Mut, ir::Ty::Bool),
            reference(ir::BorrowKind::Mut, ir::Ty::Char),
            reference(ir::BorrowKind::Shared, ir::Ty::Int(ir::IntTy::I32)),
        ];
        let trace = infer(own, &parameters, &inputs, &budget).unwrap();
        let erased: CallTrace = trace.clone().into();
        for (argument, expectation) in [
            ir::CallExpectation::NoFirmReference,
            ir::CallExpectation::FirmReference {
                kind: ir::BorrowKind::Mut,
            },
            ir::CallExpectation::FirmReference {
                kind: ir::BorrowKind::Shared,
            },
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(trace.expected(argument, &budget).unwrap(), expectation);
            assert_eq!(erased.expected(argument, &budget).unwrap(), expectation);
        }
        assert!(trace.expected(3, &budget).is_err());
        assert!(erased.expected(usize::MAX, &budget).is_err());
        let mut malformed = (*trace).clone();
        malformed.evidence = Arc::from([]);
        assert!(malformed.expected(0, &budget).is_err());
    }

    #[test]
    fn full_input_trace_replays_multiple_slots_and_nested_aggregates() {
        let budget = ir::PlanBudget::default();
        let own = env(3);
        let caller = env(1);
        let rigid = param(&caller, 0);
        let parameters = [
            reference(
                ir::BorrowKind::Shared,
                ir::Ty::Tuple(Arc::from([
                    array(param(&own, 0), 4),
                    reference(ir::BorrowKind::Mut, param(&own, 1)),
                ])),
            ),
            array(reference(ir::BorrowKind::Shared, param(&own, 2)), 2),
            param(&own, 0),
        ];
        let inputs = [
            reference(
                ir::BorrowKind::Shared,
                ir::Ty::Tuple(Arc::from([
                    array(ir::Ty::Bool, 4),
                    reference(ir::BorrowKind::Mut, rigid.clone()),
                ])),
            ),
            array(reference(ir::BorrowKind::Shared, ir::Ty::Char), 2),
            ir::Ty::Bool,
        ];
        let trace = infer(own.clone(), &parameters, &inputs, &budget).unwrap();
        let replay = infer(own.clone(), &parameters, &inputs, &budget).unwrap();
        assert_eq!(
            trace.actuals(),
            &[ir::Ty::Bool, rigid.clone(), ir::Ty::Char]
        );
        trace.validate_replayed(&replay, &budget).unwrap();
        trace.validate_actuals(trace.actuals(), &budget).unwrap();
        assert!(
            trace
                .validate_actuals(&[ir::Ty::Bool, ir::Ty::Char, rigid], &budget)
                .is_err()
        );
        let mut conflict = inputs.clone();
        conflict[2] = ir::Ty::Char;
        assert!(infer(own.clone(), &parameters, &conflict, &budget).is_err());
        assert!(infer(own, &parameters, &inputs[..2], &budget).is_err());

        let mut changed = (*replay).clone();
        changed.schemas = Arc::from([]);
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.evidence = Arc::from([]);
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.slots = Arc::from([]);
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.actuals = Arc::from([ir::Ty::Char, ir::Ty::Bool, ir::Ty::Char]);
        assert!(trace.validate_replayed(&changed, &budget).is_err());
        let mut changed = (*replay).clone();
        changed.result_schema = Arc::new(InferKind::Rigid(ir::Ty::Bool));
        assert!(trace.validate_replayed(&changed, &budget).is_err());
    }

    #[test]
    fn full_input_trace_preserves_rigid_binders_and_reference_kinds() {
        let budget = ir::PlanBudget::default();
        let own = env(2);
        let caller = env(1);
        let other_caller = env(1);
        let parameters = [param(&own, 0), param(&own, 1), param(&own, 0)];
        let rigid = param(&caller, 0);
        let inputs = [
            rigid.clone(),
            reference(ir::BorrowKind::Mut, ir::Ty::Char),
            rigid,
        ];
        let trace = infer(own.clone(), &parameters, &inputs, &budget).unwrap();
        let mut foreign = inputs.clone();
        foreign[2] = param(&other_caller, 0);
        assert!(infer(own.clone(), &parameters, &foreign, &budget).is_err());
        let mut changed = inputs.clone();
        changed[1] = reference(ir::BorrowKind::Shared, ir::Ty::Char);
        let changed = infer(own, &parameters, &changed, &budget).unwrap();
        assert!(trace.validate_replayed(&changed, &budget).is_err());
    }

    #[test]
    fn full_input_trace_keeps_caller_schema_rigid_and_result_non_inferential() {
        let budget = ir::PlanBudget::default();
        let own = env(1);
        let caller = env(1);
        let rigid = param(&caller, 0);
        let parameters = [
            param(&own, 0),
            reference(ir::BorrowKind::Shared, rigid.clone()),
        ];
        let inputs = [
            ir::Ty::Char,
            reference(ir::BorrowKind::Shared, rigid.clone()),
        ];
        let trace = infer(own.clone(), &parameters, &inputs, &budget).unwrap();
        assert_eq!(trace.actuals(), &[ir::Ty::Char]);
        let changed = [
            ir::Ty::Char,
            reference(ir::BorrowKind::Shared, ir::Ty::Bool),
        ];
        assert!(infer(own.clone(), &parameters, &changed, &budget).is_err());

        // Even a result schema naming the slot is not independently authored
        // expected-type evidence. An unused input slot must remain unresolved.
        assert!(
            infer_input_schema_trace(
                Some(own.clone()),
                &[ir::Ty::Bool],
                &param(&own, 0),
                &[ir::Ty::Bool],
                &budget,
                &mut |_, _| unreachable!()
            )
            .is_err()
        );
    }

    #[test]
    fn full_input_trace_accepts_the_entire_own_slot_range_in_declaration_order() {
        let budget = ir::PlanBudget::default();
        let own = env(32);
        let parameters: Vec<_> = (0..32).rev().map(|i| param(&own, i)).collect();
        let inputs: Vec<_> = (0..32)
            .rev()
            .map(|i| {
                if i % 2 == 0 {
                    ir::Ty::Bool
                } else {
                    ir::Ty::Char
                }
            })
            .collect();
        let trace = infer(own, &parameters, &inputs, &budget).unwrap();
        for (index, actual) in trace.actuals().iter().enumerate() {
            assert_eq!(
                *actual,
                if index % 2 == 0 {
                    ir::Ty::Bool
                } else {
                    ir::Ty::Char
                }
            );
        }
    }

    #[test]
    fn full_input_trace_rejects_unconstrained_and_malformed_slots_and_is_metered() {
        let budget = ir::PlanBudget::default();
        let own = env(2);
        assert!(infer(own.clone(), &[param(&own, 0)], &[ir::Ty::Bool], &budget).is_err());
        assert!(infer(own.clone(), &[param(&own, 2)], &[ir::Ty::Bool], &budget).is_err());
        let tiny = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        let err = infer(own.clone(), &[param(&own, 0)], &[ir::Ty::Bool], &tiny).unwrap_err();
        assert!(err.is_resource());
        let trace = infer_input_schema_trace(
            None,
            &[ir::Ty::Char],
            &ir::Ty::Unit,
            &[ir::Ty::Char],
            &budget,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert!(trace.actuals().is_empty());
    }

    #[test]
    fn nominal_constraints_match_identity_and_ordered_nested_arguments() {
        let budget = ir::PlanBudget::default();
        let own = env(2);
        let mut state = CallInferenceState::new(Some(own.clone()), &budget).unwrap();
        let declaration = own.binder().declaration;
        let schema = state
            .node(InferKind::Nominal(
                declaration,
                Arc::from([
                    state.schema(&array(param(&own, 1), 3), 0).unwrap(),
                    state
                        .schema(&reference(ir::BorrowKind::Shared, param(&own, 0)), 0)
                        .unwrap(),
                ]),
            ))
            .unwrap();
        let evidence = state
            .node(InferKind::Nominal(
                declaration,
                Arc::from([
                    state.type_term(&array(ir::Ty::Char, 3), None, 0).unwrap(),
                    state
                        .type_term(&reference(ir::BorrowKind::Shared, ir::Ty::Bool), None, 0)
                        .unwrap(),
                ]),
            ))
            .unwrap();
        state.constrain(&schema, &evidence, 0).unwrap();
        assert_eq!(
            state
                .finish(false, &mut |_, _| unreachable!())
                .unwrap()
                .unwrap(),
            vec![ir::Ty::Bool, ir::Ty::Char]
        );
        let other = env(0).binder().declaration;
        let InferKind::Nominal(_, args) = evidence.as_ref() else {
            unreachable!()
        };
        let foreign = state.node(InferKind::Nominal(other, args.clone())).unwrap();
        assert!(state.constrain(&schema, &foreign, 0).is_err());
        assert!(!terms_equal(&evidence, &foreign, &budget, 0).unwrap());
    }
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
                snapshot: ir::SnapshotId::standalone().unwrap(),
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

        let legacy: CallTrace = Arc::new(mutable).into();
        assert_eq!(
            legacy.expected(0, &budget).unwrap(),
            ir::CallExpectation::NoFirmReference
        );
        assert!(legacy.expected(1, &budget).is_err());
        let replaced: CallTrace = Arc::new(replaced).into();
        assert!(replaced.expected(0, &budget).is_err());
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

impl crate::engine::Fact for CanonicalSourcePublication {}
