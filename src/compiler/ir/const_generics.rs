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

/// Fixed scalar order for canonical const argument identities.
const KEY_INTS: [IntTy; 12] = [
    IntTy::I8,
    IntTy::I16,
    IntTy::I32,
    IntTy::I64,
    IntTy::I128,
    IntTy::Isize,
    IntTy::U8,
    IntTy::U16,
    IntTy::U32,
    IntTy::U64,
    IntTy::U128,
    IntTy::Usize,
];

impl ConstArgument {
    /// The declared scalar type of a closed argument.
    pub fn closed_type(&self) -> Option<ConstType> {
        match self {
            Self::Parameter(_) => None,
            Self::Bool(_) => Some(ConstType::Bool),
            Self::Char(_) => Some(ConstType::Char),
            Self::Integer { ty, .. } => Some(ConstType::Int(*ty)),
        }
    }

    /// A closed `usize` value, the only admitted array extent.
    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Self::Integer {
                ty: IntTy::Usize,
                bits,
            } => usize::try_from(*bits).ok(),
            _ => None,
        }
    }

    /// Canonical identity of a closed argument: scalar kind, scalar type and
    /// value bits. Symbolic parameters have no instance identity.
    pub fn closed_key(&self) -> Result<[u64; 4], String> {
        Ok(match self {
            Self::Parameter(_) => {
                return Err("const argument identity requires a closed value".into());
            }
            Self::Bool(value) => [0, 0, u64::from(*value), 0],
            Self::Char(value) => [1, 0, u64::from(u32::from(*value)), 0],
            Self::Integer { ty, bits } => {
                if *bits & !ty.mask() != 0 {
                    return Err(
                        "const argument integer payload exceeds its declared scalar type".into(),
                    );
                }
                let index = KEY_INTS
                    .iter()
                    .position(|candidate| candidate == ty)
                    .ok_or("unknown const argument integer type")?;
                [2, index as u64, *bits as u64, (*bits >> 64) as u64]
            }
        })
    }

    pub fn from_closed_key(words: [u64; 4]) -> Result<Self, String> {
        let value = match words {
            [0, 0, value @ (0 | 1), 0] => Self::Bool(value == 1),
            [1, 0, value, 0] => Self::Char(
                u32::try_from(value)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("invalid const argument char identity")?,
            ),
            [2, index, low, high] => {
                let ty = *usize::try_from(index)
                    .ok()
                    .and_then(|index| KEY_INTS.get(index))
                    .ok_or("invalid const argument integer identity")?;
                Self::Integer {
                    ty,
                    bits: u128::from(low) | (u128::from(high) << 64),
                }
            }
            _ => return Err("invalid const argument identity".into()),
        };
        if value.closed_key()? != words {
            return Err("noncanonical const argument identity".into());
        }
        Ok(value)
    }
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
