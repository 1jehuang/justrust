//! Pure parent-sealed source witnesses. No Ty, signature, body or world is retained.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResultTypeWitness {
    Parameter(ParamId),
    Unit,
    SharedStr,
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
    Required {
        item: DeclId,
        trait_decl: DeclId,
        self_ty: ResultTypeWitness,
        trait_args: Arc<[ResultTypeWitness]>,
        method_args: Arc<[ResultTypeWitness]>,
    },
    Argument {
        declaration: DeclId,
        origin: ClosureArgumentOrigin,
    },
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
                declaration,
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
    pub(in crate::compiler) fn argument_callee(
        &self,
        budget: &PlanBudget,
    ) -> Result<Option<Callee>, String> {
        match &self.call {
            ResultCallWitness::Argument { declaration, .. } => {
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
