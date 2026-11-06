//! Immutable declaration-owned const binders. No spelling is a semantic identity.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstParamId {
    pub owner: BinderId,
    pub index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConstType {
    Int(IntTy),
    Bool,
    Char,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConstArgument {
    Parameter(ConstParamId),
    Bool(bool),
    Char(char),
    Integer { ty: IntTy, bits: u128 },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GenericArgument {
    Type(Ty),
    Const(ConstArgument),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstBinder {
    owner: BinderId,
    parameters: Arc<[ConstType]>,
}

impl ConstBinder {
    pub(in crate::compiler) fn new(
        owner: BinderId,
        parameters: &[ConstType],
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        if parameters.len() > budget.limits().params {
            return Err("const parameter resource limit exceeded".into());
        }
        budget.charge_work(parameters.len() + 1)?;
        budget.retain_bytes(std::mem::size_of::<Self>() + std::mem::size_of_val(parameters))?;
        Ok(Self {
            owner,
            parameters: parameters.into(),
        })
    }

    pub fn owner(&self) -> BinderId {
        self.owner
    }
    pub fn parameters(&self) -> &[ConstType] {
        &self.parameters
    }
    pub fn parameter_type(&self, parameter: ConstParamId) -> Option<ConstType> {
        (parameter.owner == self.owner)
            .then(|| self.parameters.get(usize::from(parameter.index)).copied())
            .flatten()
    }

    pub fn validate_arguments(
        &self,
        arguments: &[GenericArgument],
        caller: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(arguments.len().saturating_add(self.parameters.len()))?;
        if arguments.len() != self.parameters.len() {
            return Err("E0107: wrong number of const generic arguments".into());
        }
        for (argument, expected) in arguments.iter().zip(self.parameters.iter()) {
            let GenericArgument::Const(argument) = argument else {
                return Err("E0747: type provided when a constant was expected".into());
            };
            let actual = match argument {
                ConstArgument::Parameter(parameter) => caller
                    .const_parameter_type(*parameter)
                    .ok_or("const argument is outside its declaration-owned binder")?,
                ConstArgument::Bool(_) => ConstType::Bool,
                ConstArgument::Char(_) => ConstType::Char,
                ConstArgument::Integer { ty, bits } => {
                    if *bits & !ty.mask() != 0 {
                        return Err(
                            "const argument integer payload exceeds its declared scalar type"
                                .into(),
                        );
                    }
                    ConstType::Int(*ty)
                }
            };
            if actual != *expected {
                return Err(
                    "E0308: const generic argument type does not match declaration parameter type"
                        .into(),
                );
            }
        }
        Ok(())
    }
}
