//! Declaration-owned const values and symbolic repeat extents. A const slot's
//! value type is its scalar type, never the generic-argument carrier `Ty::Const`.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstOperand {
    parameter: ConstParamId,
    scalar: ConstType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstValue {
    Closed(ConstArgument),
    Parameter(ConstOperand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepeatLength {
    Known(usize),
    Parameter(ConstOperand),
}

impl ConstType {
    pub fn value_type(self) -> Ty {
        match self {
            Self::Int(ty) => Ty::Int(ty),
            Self::Bool => Ty::Bool,
            Self::Char => Ty::Char,
        }
    }
}

fn caller_environment<'a>(facts: FactsCtx<'a>) -> Option<&'a GenericEnv> {
    match facts {
        FactsCtx::Concrete => None,
        FactsCtx::Universal(env) => Some(env),
        FactsCtx::Validated(facts) => Some(facts.env().as_ref()),
    }
}

impl ConstOperand {
    pub(in crate::compiler) fn new(
        parameter: ConstParamId,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.charge_work(1)?;
        let env = caller_environment(facts)
            .ok_or("const parameter value requires an authoritative generic environment")?;
        budget.charge_work(env.scopes().len())?;
        let slot = ParamId {
            owner: parameter.owner,
            index: parameter.index,
        };
        let Some(GenericParamKind::Const(scalar)) = env.parameter_kind(slot) else {
            return Err("const parameter value is outside its declaration-owned const slot".into());
        };
        Ok(Self { parameter, scalar })
    }

    pub fn parameter(self) -> ConstParamId {
        self.parameter
    }
    pub fn scalar_type(self) -> ConstType {
        self.scalar
    }
    pub fn value_type(self) -> Ty {
        self.scalar.value_type()
    }

    pub(in crate::compiler) fn validate(
        self,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if Self::new(self.parameter, facts, budget)? != self {
            return Err("const parameter value changed its declared scalar type".into());
        }
        Ok(())
    }

    /// Substitute once under the callee binder. Forwarded parameters remain
    /// rigid caller values, including recursive calls with the same owner.
    pub(in crate::compiler) fn substitute(
        self,
        env: &GenericEnv,
        arguments: &[Ty],
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<ConstValue, String> {
        env.validate_argument_kinds(arguments, caller_environment(caller), budget)?;
        let slot = ParamId {
            owner: self.parameter.owner,
            index: self.parameter.index,
        };
        let Some(index) = env.argument_index(slot) else {
            self.validate(caller, budget)?;
            return Ok(ConstValue::Parameter(self));
        };
        if env.parameter_kind(slot) != Some(GenericParamKind::Const(self.scalar)) {
            return Err("const value substitution changed the declaration slot kind".into());
        }
        match arguments.get(index) {
            Some(Ty::Const(value)) => {
                value.closed_key()?;
                if value.closed_type() != Some(self.scalar) {
                    return Err("E0308: const value argument has the wrong scalar type".into());
                }
                Ok(ConstValue::Closed(value.clone()))
            }
            Some(Ty::Param(parameter)) => {
                let forwarded = Self::new(
                    ConstParamId {
                        owner: parameter.owner,
                        index: parameter.index,
                    },
                    caller,
                    budget,
                )?;
                if forwarded.scalar != self.scalar {
                    return Err("E0308: forwarded const value has the wrong scalar type".into());
                }
                Ok(ConstValue::Parameter(forwarded))
            }
            _ => Err("E0747: expected a const value argument".into()),
        }
    }
}

impl ConstValue {
    pub fn value_type(&self) -> Result<Ty, String> {
        match self {
            Self::Parameter(value) => Ok(value.value_type()),
            Self::Closed(value) => {
                value.closed_key()?;
                Ok(value
                    .closed_type()
                    .ok_or("const value is not closed")?
                    .value_type())
            }
        }
    }

    /// Only a closed value can become an executable scalar literal. Symbolic
    /// values must first be specialized by the owning function instance.
    pub(in crate::compiler) fn closed_expression(&self) -> Result<TypedExpr, String> {
        let Self::Closed(value) = self else {
            return Err(
                "unsupported: unspecialized const value reached executable lowering".into(),
            );
        };
        value.closed_key()?;
        let kind = match value {
            ConstArgument::Integer { ty, bits } => ExprKind::Int(IntValue {
                ty: *ty,
                bits: *bits,
            }),
            ConstArgument::Bool(value) => ExprKind::Bool(*value),
            ConstArgument::Char(value) => ExprKind::Char(CharValue {
                value: u32::from(*value),
            }),
            ConstArgument::Parameter(_) => {
                return Err("const scalar literal requires a closed value".into());
            }
        };
        Ok(TypedExpr {
            ty: self.value_type()?,
            completion: Completion::Returns,
            kind,
        })
    }
}

impl From<usize> for RepeatLength {
    fn from(len: usize) -> Self {
        Self::Known(len)
    }
}

impl RepeatLength {
    pub(in crate::compiler) fn parameter(
        operand: ConstOperand,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        operand.validate(facts, budget)?;
        if operand.scalar != ConstType::Int(IntTy::Usize) {
            return Err("E0308: array repeat length must have type usize".into());
        }
        Ok(Self::Parameter(operand))
    }

    pub fn requires_copy(self) -> bool {
        !matches!(self, Self::Known(0 | 1))
    }

    pub(in crate::compiler) fn array_type(
        self,
        element: Ty,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        budget.charge_work(1)?;
        facts.validate_ty_with_budget(&element, budget)?;
        if self.requires_copy() && !facts.must_copy(&element)? {
            return Err("array repeat of potentially more than one element requires Copy".into());
        }
        budget.retain_bytes(std::mem::size_of::<Ty>())?;
        match self {
            Self::Known(len) => {
                if len > MAX_ARRAY_TYPE_LEN {
                    return Err("array repeat exceeds supported length".into());
                }
                Ok(Ty::Array {
                    element: Arc::new(element),
                    len,
                })
            }
            Self::Parameter(operand) => {
                Self::parameter(operand, facts, budget)?;
                Ok(Ty::DependentArray {
                    element: Arc::new(element),
                    length: operand.parameter,
                })
            }
        }
    }

    pub(in crate::compiler) fn substitute(
        self,
        env: &GenericEnv,
        arguments: &[Ty],
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        match self {
            Self::Known(len) => {
                if len > MAX_ARRAY_TYPE_LEN {
                    return Err("array repeat exceeds supported length".into());
                }
                Ok(self)
            }
            Self::Parameter(operand) => match operand.substitute(env, arguments, caller, budget)? {
                ConstValue::Closed(value) => {
                    let len = value
                        .as_usize()
                        .ok_or("E0308: array repeat length must be usize")?;
                    if len > MAX_ARRAY_TYPE_LEN {
                        return Err("array repeat exceeds supported length".into());
                    }
                    Ok(Self::Known(len))
                }
                ConstValue::Parameter(operand) => Self::parameter(operand, caller, budget),
            },
        }
    }
}
