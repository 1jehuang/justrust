//! Pure parent-sealed source witnesses. No Ty, signature, body or world is retained.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResultTypeWitness {
    Parameter(ParamId),
    Unit,
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
    item: DeclId,
    trait_decl: DeclId,
    self_ty: ResultTypeWitness,
    trait_args: Arc<[ResultTypeWitness]>,
    method_args: Arc<[ResultTypeWitness]>,
    result: ResultTypeWitness,
    arguments: Arc<[ExprId]>,
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
            result: ResultTypeWitness::from_ty(&signature.result, budget, 0)?,
            arguments,
        })
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
        Ok(TraitCall {
            item: self.item,
            trait_ref: TraitRef {
                declaration: self.trait_decl,
                self_ty: self.self_ty.realize(binding, budget, 0)?,
                args: self
                    .trait_args
                    .iter()
                    .map(|ty| ty.realize(binding, budget, 0))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            },
            method_args: self
                .method_args
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
