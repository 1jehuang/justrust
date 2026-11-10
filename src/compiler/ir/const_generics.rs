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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericParamKind {
    Type,
    Const(ConstType),
}

impl ParamEnv {
    /// Preserve the declaration's source order across type and const slots.
    /// This does not add slots or manufacture a second binder identity.
    pub(in crate::compiler) fn with_slot_kinds(
        &self,
        kinds: &[GenericParamKind],
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.charge_work(kinds.len().saturating_add(1))?;
        if kinds.len() != self.arity() {
            return Err("generic slot kinds do not match binder arity".into());
        }
        for (index, kind) in kinds.iter().enumerate() {
            if matches!(kind, GenericParamKind::Const(_))
                && self.relaxed_sized & (1u32 << index) != 0
            {
                return Err("const parameter cannot have a relaxed Sized bound".into());
            }
        }
        let kinds = if kinds.iter().all(|kind| *kind == GenericParamKind::Type) {
            None
        } else {
            budget.retain_bytes(std::mem::size_of_val(kinds))?;
            Some(Arc::from(kinds))
        };
        Ok(Self {
            kinds,
            ..self.clone()
        })
    }

    pub fn parameter_kind(&self, parameter: ParamId) -> Option<GenericParamKind> {
        if !self.contains(parameter) {
            return None;
        }
        Some(self.kinds.as_ref().map_or(GenericParamKind::Type, |kinds| {
            kinds[usize::from(parameter.index)]
        }))
    }

    pub fn same_slot_kinds(&self, other: &Self) -> bool {
        self.arity() == other.arity() && self.kinds == other.kinds
    }

    pub(in crate::compiler) fn validate_argument_kinds(
        &self,
        arguments: &[Ty],
        caller: Option<&GenericEnv>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(arguments.len().saturating_add(1))?;
        if arguments.len() != self.arity() {
            return Err("E0107: wrong number of generic arguments".into());
        }
        for (index, argument) in arguments.iter().enumerate() {
            let expected = self.parameter_kind(ParamId {
                owner: self.binder(),
                index: index as u16,
            });
            let actual = match argument {
                Ty::Const(value) => {
                    value.closed_key()?;
                    Some(GenericParamKind::Const(
                        value.closed_type().ok_or("const argument must be closed")?,
                    ))
                }
                Ty::Param(parameter) => caller.and_then(|env| env.parameter_kind(*parameter)),
                _ => Some(GenericParamKind::Type),
            };
            match (expected, actual) {
                (Some(GenericParamKind::Type), Some(GenericParamKind::Type)) => {}
                (Some(GenericParamKind::Const(a)), Some(GenericParamKind::Const(b))) if a == b => {}
                (Some(GenericParamKind::Type), Some(GenericParamKind::Const(_))) => {
                    return Err("E0747: constant provided when a type was expected".into());
                }
                (Some(GenericParamKind::Const(_)), Some(GenericParamKind::Type)) => {
                    return Err("E0747: type provided when a constant was expected".into());
                }
                (Some(GenericParamKind::Const(_)), Some(GenericParamKind::Const(_))) => {
                    return Err("E0308: const argument has the wrong scalar type".into());
                }
                _ => return Err("generic argument is outside its authoritative binder".into()),
            }
        }
        Ok(())
    }
}

impl GenericEnv {
    /// Recover the exact source-owned usize coordinate, without substituting
    /// or interpreting any other slot. Callers still authenticate a forwarded
    /// parameter against its own source environment.
    pub(in crate::compiler) fn array_length_argument<'a>(
        &self,
        parameter: ConstParamId,
        arguments: &'a [Ty],
        budget: &PlanBudget,
    ) -> Result<&'a Ty, String> {
        budget.charge_work(self.scopes().len().saturating_add(1))?;
        if self.const_parameter_type(parameter) != Some(ConstType::Int(IntTy::Usize)) {
            return Err("dependent array has no authoritative usize parameter".into());
        }
        if arguments.len() != self.total_arity() {
            return Err("dependent array substitution changed ordered argument arity".into());
        }
        let index = self
            .argument_index(ParamId {
                owner: parameter.owner,
                index: parameter.index,
            })
            .ok_or("const argument has no ordered slot")?;
        arguments
            .get(index)
            .ok_or_else(|| "missing const array length argument".into())
    }

    /// Substitute only the callee's extent. A forwarded caller parameter is
    /// rigid and must not be substituted again, even when binders coincide.
    pub(super) fn substitute_array_length(
        &self,
        parameter: ConstParamId,
        arguments: &[Ty],
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<ConstArgument, String> {
        budget.charge_work(self.scopes().len().saturating_add(1))?;
        if self
            .scopes()
            .iter()
            .any(|scope| scope.binder() == parameter.owner)
            && self.const_parameter_type(parameter) != Some(ConstType::Int(IntTy::Usize))
        {
            return Err("dependent array has no authoritative usize parameter".into());
        }
        let slot = ParamId {
            owner: parameter.owner,
            index: parameter.index,
        };
        if self.parameter_kind(slot) == Some(GenericParamKind::Const(ConstType::Int(IntTy::Usize)))
        {
            let argument = self.array_length_argument(parameter, arguments, budget)?;
            match argument {
                Ty::Const(value) => {
                    value.closed_key()?;
                    let len = value
                        .as_usize()
                        .ok_or("E0308: array length requires usize")?;
                    if len > MAX_ARRAY_TYPE_LEN {
                        return Err("array length limit exceeded".into());
                    }
                    return Ok(value.clone());
                }
                Ty::Param(actual) => {
                    let caller_env = match caller {
                        FactsCtx::Universal(env) => Some(env),
                        FactsCtx::Validated(facts) => Some(facts.env().as_ref()),
                        FactsCtx::Concrete => None,
                    };
                    if caller_env.and_then(|env| env.parameter_kind(*actual))
                        != Some(GenericParamKind::Const(ConstType::Int(IntTy::Usize)))
                    {
                        return Err(
                            "E0308: array length requires an authoritative usize parameter".into(),
                        );
                    }
                    return Ok(ConstArgument::Parameter(ConstParamId {
                        owner: actual.owner,
                        index: actual.index,
                    }));
                }
                _ => return Err("E0747: expected a const array length argument".into()),
            }
        }
        // Legacy source-only const binders are not instance coordinates yet.
        // Preserve them only when the caller owns the same symbolic extent.
        let caller_type = match caller {
            FactsCtx::Universal(env) => env.const_parameter_type(parameter),
            FactsCtx::Validated(facts) => facts.env().const_parameter_type(parameter),
            FactsCtx::Concrete => None,
        };
        if caller_type != Some(ConstType::Int(IntTy::Usize)) {
            return Err("dependent array requires an authoritative caller const binder".into());
        }
        Ok(ConstArgument::Parameter(parameter))
    }

    pub(in crate::compiler) fn validate_argument_kinds(
        &self,
        arguments: &[Ty],
        caller: Option<&GenericEnv>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if arguments.len() != self.total_arity() {
            return Err("E0107: wrong number of generic arguments".into());
        }
        let mut offset = 0;
        for scope in self.scopes() {
            let end = offset + scope.arity();
            scope.validate_argument_kinds(&arguments[offset..end], caller, budget)?;
            offset = end;
        }
        Ok(())
    }

    pub fn parameter_kind(&self, parameter: ParamId) -> Option<GenericParamKind> {
        self.scopes()
            .iter()
            .find_map(|scope| scope.parameter_kind(parameter))
    }
}

impl Ty {
    /// Generic argument leaves have no value-type children. Closed constants
    /// are admitted only by callers walking an ordered argument list.
    pub(super) fn is_const_argument_leaf(
        &self,
        facts: Option<FactsCtx<'_>>,
    ) -> Result<bool, String> {
        match self {
            Ty::Const(value) => {
                value.closed_key()?;
                Ok(true)
            }
            Ty::Param(parameter) => {
                let kind = match facts {
                    Some(FactsCtx::Universal(env)) => env.parameter_kind(*parameter),
                    Some(FactsCtx::Validated(facts)) => facts.env().parameter_kind(*parameter),
                    _ => None,
                };
                Ok(matches!(kind, Some(GenericParamKind::Const(_))))
            }
            _ => Ok(false),
        }
    }
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
