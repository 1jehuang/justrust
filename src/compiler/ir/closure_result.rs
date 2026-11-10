//! Parent-sealed source witnesses. No signature, body or world is retained.
use super::*;
#[path = "closure_capture_result.rs"]
mod capture;
pub(in crate::compiler) use capture::*;
#[path = "closure_input_result.rs"]
mod input;
pub(in crate::compiler) use input::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResultTypeWitness {
    Parameter(ParamId),
    Unit,
    SharedStr,
    ProjectedInput,
    Captured,
    Tuple(Arc<[ResultTypeWitness]>),
}
impl ResultTypeWitness {
    fn from_ty(ty: &Ty, budget: &PlanBudget, depth: usize) -> Result<Self, String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        budget.retain_bytes(std::mem::size_of::<Self>())?;
        match ty {
            Ty::Param(id) => Ok(Self::Parameter(*id)),
            Ty::Unit => Ok(Self::Unit),
            Ty::Tuple(fields) => Ok(Self::Tuple(fields.iter().map(|ty| Self::from_ty(ty, budget, depth + 1)).collect::<Result<Vec<_>, _>>()?.into())),
            _ => Err("unsupported: closure result witness requires canonical parameter, unit or tuple source types".into()),
        }
    }
    fn realize(
        &self,
        binding: &impl Fn(ParamId) -> Result<Ty, String>,
        budget: &PlanBudget,
        depth: usize,
    ) -> Result<Ty, String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        budget.retain_bytes(std::mem::size_of::<Ty>())?;
        match self {
            Self::Parameter(id) => binding(*id),
            Self::Unit => Ok(Ty::Unit),
            Self::SharedStr => {
                budget.retain_bytes(
                    std::mem::size_of::<Ty>()
                        .checked_add(2 * std::mem::size_of::<usize>())
                        .ok_or("closure shared-str witness allocation overflow")?,
                )?;
                Ok(Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: Arc::new(Ty::Str),
                })
            }
            Self::ProjectedInput => {
                Err("projected closure result requires canonical input types".into())
            }
            Self::Captured => {
                Err("captured closure result requires original source shape replay".into())
            }
            Self::Tuple(fields) => Ok(Ty::Tuple(
                fields
                    .iter()
                    .map(|field| field.realize(binding, budget, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            )),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClosureResultReceipt {
    owner: DeclId,
    provider: DeclId,
    site: u32,
    tail: ExprId,
    call: ResultCallWitness,
    result: ResultTypeWitness,
    arguments: Arc<[ExprId]>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResultCallWitness {
    StaticStr,
    Required {
        item: DeclId,
        trait_decl: DeclId,
        self_ty: ResultTypeWitness,
        trait_args: Arc<[ResultTypeWitness]>,
        method_args: Arc<[ResultTypeWitness]>,
        inputs: Option<Arc<[ClosureInputValueOrigin]>>,
    },
    Argument {
        declaration: Option<DeclId>,
        origin: ClosureArgumentOrigin,
    },
    Captured(Arc<CaptureResultWitness>),
}
/// Parent-minted source tuple occurrence, never an inferred capture origin.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClosureArgumentOrigin {
    pub(in crate::compiler) argument: usize,
    pub(in crate::compiler) local: LocalId,
    pub(in crate::compiler) tuple: LocalId,
    pub(in crate::compiler) borrow: ExprId,
    pub(in crate::compiler) place: Place,
    pub(in crate::compiler) depth: u8,
}
impl ClosureResultReceipt {
    pub(in crate::compiler) fn static_str(
        owner: DeclId,
        provider: DeclId,
        site: u32,
        tail: ExprId,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        Ok(Self {
            owner,
            provider,
            site,
            tail,
            call: ResultCallWitness::StaticStr,
            result: ResultTypeWitness::SharedStr,
            arguments: Arc::from([]),
        })
    }
    pub(in crate::compiler) fn is_static_str(&self) -> bool {
        matches!(self.call, ResultCallWitness::StaticStr)
    }
    pub(in crate::compiler) fn captured(
        owner: DeclId,
        provider: DeclId,
        site: u32,
        tail: ExprId,
        witness: CaptureResultWitness,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.retain_bytes(
            std::mem::size_of::<Self>() + std::mem::size_of::<CaptureResultWitness>() + 32,
        )?;
        Ok(Self {
            owner,
            provider,
            site,
            tail,
            call: ResultCallWitness::Captured(Arc::new(witness)),
            result: ResultTypeWitness::Captured,
            arguments: Arc::from([]),
        })
    }
    pub(in crate::compiler) fn capture_witness(&self) -> Option<&CaptureResultWitness> {
        match &self.call {
            ResultCallWitness::Captured(witness) => Some(witness),
            _ => None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn new(
        owner: DeclId,
        provider: DeclId,
        site: u32,
        tail: ExprId,
        call: TraitCall,
        arguments: Arc<[ExprId]>,
        signature: Signature,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        Ok(Self {
            owner,
            provider,
            site,
            tail,
            call: ResultCallWitness::Required {
                item: call.item,
                trait_decl: call.trait_ref.declaration,
                self_ty: ResultTypeWitness::from_ty(&call.trait_ref.self_ty, budget, 0)?,
                trait_args: call
                    .trait_ref
                    .args
                    .iter()
                    .map(|ty| ResultTypeWitness::from_ty(ty, budget, 0))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
                method_args: call
                    .method_args
                    .iter()
                    .map(|ty| ResultTypeWitness::from_ty(ty, budget, 0))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
                inputs: None,
            },
            result: ResultTypeWitness::from_ty(&signature.result, budget, 0)?,
            arguments,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn argument_str(
        owner: DeclId,
        provider: DeclId,
        site: u32,
        tail: ExprId,
        declaration: DeclId,
        argument: usize,
        local: LocalId,
        tuple: LocalId,
        borrow: ExprId,
        place: Place,
        depth: u8,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        let bytes = place
            .projection
            .len()
            .checked_mul(std::mem::size_of::<Projection>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ExprId>()))
            // Receipt Arc plus its exact one-argument Arc slice.
            .and_then(|bytes| bytes.checked_add(4 * std::mem::size_of::<usize>()))
            .ok_or("closure argument result witness allocation overflow")?;
        budget.retain_bytes(bytes)?;
        Ok(Self {
            owner,
            provider,
            site,
            tail,
            call: ResultCallWitness::Argument {
                declaration: Some(declaration),
                origin: ClosureArgumentOrigin {
                    argument,
                    local,
                    tuple,
                    borrow,
                    place,
                    depth,
                },
            },
            result: ResultTypeWitness::SharedStr,
            arguments: Arc::from([borrow]),
        })
    }
    pub(in crate::compiler) fn argument_origin(&self) -> Option<&ClosureArgumentOrigin> {
        match &self.call {
            ResultCallWitness::Argument { origin, .. } => Some(origin),
            _ => None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn projected_input(
        owner: DeclId,
        provider: DeclId,
        site: u32,
        tail: ExprId,
        argument: usize,
        tuple: LocalId,
        borrow: ExprId,
        place: Place,
        depth: u8,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        Ok(Self {
            owner,
            provider,
            site,
            tail,
            call: ResultCallWitness::Argument {
                declaration: None,
                origin: ClosureArgumentOrigin {
                    argument,
                    local: place.local,
                    tuple,
                    borrow,
                    place,
                    depth,
                },
            },
            result: ResultTypeWitness::ProjectedInput,
            arguments: Arc::from([]),
        })
    }
    pub(in crate::compiler) fn is_projected_input(&self) -> bool {
        matches!(
            self.call,
            ResultCallWitness::Argument {
                declaration: None,
                ..
            }
        )
    }
    /// Derive the result from the exact canonical argument, never a retained Ty.
    pub(in crate::compiler) fn projected_result(
        &self,
        params: &[Ty],
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        if !self.is_projected_input() {
            return Err("closure result is not a projected input".into());
        }
        let origin = self
            .argument_origin()
            .ok_or("missing projected input origin")?;
        let mut ty = params
            .get(origin.argument)
            .ok_or("foreign projected input argument")?
            .clone();
        let mut derefs = 0usize;
        for (position, projection) in origin.place.projection.iter().enumerate() {
            budget.charge_work(1)?;
            match (projection, &ty) {
                (
                    Projection::Deref,
                    Ty::Ref {
                        kind: BorrowKind::Shared,
                        referent,
                    },
                ) if position == derefs => {
                    derefs += 1;
                    ty = referent.as_ref().clone();
                }
                (Projection::Field(index), _) if derefs != 0 => {
                    ty = ty
                        .field_ty(*index, budget)?
                        .ok_or("invalid projected input field")?;
                    if matches!(ty, Ty::Ref { .. }) {
                        return Err("unsupported: closure projected reference-valued field".into());
                    }
                }
                _ => return Err("unsupported: closure projected input path".into()),
            }
        }
        if derefs != usize::from(origin.depth) + 1 {
            return Err("projected closure result changed reference depth".into());
        }
        if !carrier_locations(&ty, budget)?.is_empty() {
            return Err("unsupported: closure projected result contains nested carriers".into());
        }
        budget.retain_bytes(std::mem::size_of::<Ty>() + 2 * std::mem::size_of::<usize>())?;
        Ok(Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        })
    }
    pub(in crate::compiler) fn argument_callee(
        &self,
        budget: &PlanBudget,
    ) -> Result<Option<Callee>, String> {
        match &self.call {
            ResultCallWitness::Argument {
                declaration: Some(declaration),
                ..
            } => {
                budget.retain_bytes(2 * std::mem::size_of::<usize>())?;
                Ok(Some(Callee::Direct(CallTarget {
                    declaration: *declaration,
                    type_args: Arc::from([]),
                })))
            }
            _ => Ok(None),
        }
    }
    pub fn owner(&self) -> DeclId {
        self.owner
    }
    pub fn provider(&self) -> DeclId {
        self.provider
    }
    pub fn site(&self) -> u32 {
        self.site
    }
    pub fn tail(&self) -> ExprId {
        self.tail
    }
    pub fn arguments(&self) -> &[ExprId] {
        &self.arguments
    }
    pub fn same_source_shape(&self, other: &Self) -> bool {
        self == other
    }

    pub(in crate::compiler) fn with_input_origins(
        mut self,
        origins: Arc<[ClosureInputValueOrigin]>,
    ) -> Result<Self, String> {
        let ResultCallWitness::Required { inputs, .. } = &mut self.call else {
            return Err("closure input call requires an original required-call witness".into());
        };
        if origins.len() != self.arguments.len()
            || origins
                .iter()
                .zip(self.arguments.iter())
                .any(|(origin, value)| origin.value != *value)
        {
            return Err("closure input call changed its ordered argument occurrences".into());
        }
        *inputs = Some(origins);
        Ok(self)
    }

    pub(in crate::compiler) fn input_call_origins(&self) -> Option<&[ClosureInputValueOrigin]> {
        match &self.call {
            ResultCallWitness::Required { inputs, .. } => inputs.as_deref(),
            _ => None,
        }
    }
    pub(in crate::compiler) fn realize_call(
        &self,
        binding: &impl Fn(ParamId) -> Result<Ty, String>,
        budget: &PlanBudget,
    ) -> Result<TraitCall, String> {
        let ResultCallWitness::Required {
            item,
            trait_decl,
            self_ty,
            trait_args,
            method_args,
            ..
        } = &self.call
        else {
            return Err("closure argument result is not a required-call witness".into());
        };
        Ok(TraitCall {
            item: *item,
            trait_ref: TraitRef {
                declaration: *trait_decl,
                self_ty: self_ty.realize(binding, budget, 0)?,
                args: trait_args
                    .iter()
                    .map(|ty| ty.realize(binding, budget, 0))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            },
            method_args: method_args
                .iter()
                .map(|ty| ty.realize(binding, budget, 0))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        })
    }
    pub(in crate::compiler) fn realize_result(
        &self,
        binding: &impl Fn(ParamId) -> Result<Ty, String>,
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        self.result.realize(binding, budget, 0)
    }
}
