//! Common member proof interfaces. Inherent owners never fake trait membership.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstantMemberType {
    Trait(AssociatedConstantType),
    Inherent(InherentConstantType),
}
impl ConstantMemberType {
    pub fn member(&self) -> DeclId {
        match self {
            Self::Trait(value) => value.member(),
            Self::Inherent(value) => value.member(),
        }
    }
    pub fn ty(&self) -> &Ty {
        match self {
            Self::Trait(value) => value.ty(),
            Self::Inherent(value) => value.ty(),
        }
    }
    pub fn application(&self) -> Option<&TraitRef> {
        match self {
            Self::Trait(value) => Some(value.application()),
            Self::Inherent(_) => None,
        }
    }
    pub fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        match self {
            Self::Trait(value) => value.validate_for(facts),
            Self::Inherent(value) => value.validate_for(facts),
        }
    }
}
#[derive(Clone, Debug)]
pub enum SelectedAssociatedConstant {
    Trait(ClosedAssociatedConstant),
    Inherent(InherentConstantType),
}
impl SelectedAssociatedConstant {
    pub fn member(&self) -> DeclId {
        match self {
            Self::Trait(value) => value.member(),
            Self::Inherent(value) => value.member(),
        }
    }
    pub fn declaration(&self) -> DeclId {
        match self {
            Self::Trait(value) => value.declaration(),
            Self::Inherent(value) => value.member(),
        }
    }
    pub fn arguments(&self) -> &[Ty] {
        match self {
            Self::Trait(value) => value.arguments(),
            Self::Inherent(value) => value.arguments(),
        }
    }
    pub fn ty(&self) -> &Ty {
        match self {
            Self::Trait(value) => value.ty(),
            Self::Inherent(value) => value.ty(),
        }
    }
    pub fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        if facts.phase() != FactsPhase::Closed {
            return Err(FactsError::invalid(
                facts.budget(),
                "associated constant value selection requires closed caller authority",
            ));
        }
        match self {
            Self::Trait(value) => value.validate_for(facts),
            Self::Inherent(value) => value.validate_for(facts),
        }
    }
}
