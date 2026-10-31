//! Closed language/catalog descriptors issued before the one name store freezes.
//! These are not source occurrences and cannot manufacture AST provenance.
use super::super::expansion_budget::{ByteLease, ExpansionBudget, Storage};
use super::super::semantic_name::BuiltinAtom;
use super::super::token_session::{SessionId, TokenError};
use super::*;

#[derive(Debug)]
pub struct ProgramSymbols {
    owner: SessionId,
    keys: [NameKey; 19],
    budget: ExpansionBudget,
    _lease: ByteLease,
}
impl ProgramSymbols {
    pub(super) fn new(
        names: &mut NameStoreBuilder,
    ) -> Result<Arc<Self>, super::super::semantic_name::NameError> {
        use BuiltinAtom::*;
        let budget = names.budget().clone();
        let lease = budget.reserve(
            Storage::Retained,
            (std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>()) as u64,
        )?;
        let owner = names.session().id();
        let keys = [
            names.catalog_key(Core, Namespace::Type)?,
            names.catalog_key(Std, Namespace::Type)?,
            names.catalog_key(Marker, Namespace::Type)?,
            names.catalog_key(CloneModule, Namespace::Type)?,
            names.catalog_key(Ops, Namespace::Type)?,
            names.catalog_key(Sized, Namespace::Type)?,
            names.catalog_key(Copy, Namespace::Type)?,
            names.catalog_key(Clone, Namespace::Type)?,
            names.catalog_key(Drop, Namespace::Type)?,
            names.catalog_key(EntryMain, Namespace::Value)?,
            names.catalog_member_key(CloneMethod)?,
            names.catalog_member_key(CloneFromMethod)?,
            names.catalog_member_key(DropMethod)?,
            names.catalog_key(PhantomData, Namespace::Type)?,
            names.catalog_key(Mem, Namespace::Type)?,
            names.catalog_key(ManuallyDrop, Namespace::Type)?,
            names.catalog_member_key(ManuallyDropNew)?,
            names.catalog_member_key(ManuallyDropIntoInner)?,
            names.catalog_member_key(ManuallyDropDrop)?,
        ];
        budget.check()?;
        Ok(Arc::new(Self {
            owner,
            keys,
            budget,
            _lease: lease,
        }))
    }
    pub fn check(&self, names: &impl NameAccess) -> Result<(), String> {
        self.budget.check().map_err(|error| {
            super::super::source::SourceError::from_token(TokenError::Budget(error)).to_string()
        })?;
        if !names.session().serves(self.owner) || !self.budget.same_authority(names.budget()) {
            return Err(
                super::super::source::SourceError::from_token(TokenError::ForeignHandle)
                    .to_string(),
            );
        }
        Ok(())
    }
    pub fn key(&self, names: &impl NameAccess, atom: BuiltinAtom) -> Result<NameKey, String> {
        self.check(names)?;
        let index = match atom {
            BuiltinAtom::Core => 0,
            BuiltinAtom::Std => 1,
            BuiltinAtom::Marker => 2,
            BuiltinAtom::CloneModule => 3,
            BuiltinAtom::Ops => 4,
            BuiltinAtom::Sized => 5,
            BuiltinAtom::Copy => 6,
            BuiltinAtom::Clone => 7,
            BuiltinAtom::Drop => 8,
            BuiltinAtom::EntryMain => 9,
            BuiltinAtom::CloneMethod => 10,
            BuiltinAtom::CloneFromMethod => 11,
            BuiltinAtom::DropMethod => 12,
            BuiltinAtom::PhantomData => 13,
            BuiltinAtom::Mem => 14,
            BuiltinAtom::ManuallyDrop => 15,
            BuiltinAtom::ManuallyDropNew => 16,
            BuiltinAtom::ManuallyDropIntoInner => 17,
            BuiltinAtom::ManuallyDropDrop => 18,
        };
        Ok(self.keys[index])
    }
    pub fn matches(
        &self,
        names: &impl NameAccess,
        name: AstIdent,
        atom: BuiltinAtom,
    ) -> Result<bool, String> {
        let expected = self.key(names, atom)?;
        Ok(names
            .key(name, expected.class())
            .map_err(native_adapter::name_diagnostic)?
            == expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_member_keys_match_real_owned_occurrences_only_in_member_class() {
        let program =
            parse("fn clone(){}fn clone_from(){}fn drop(){}fn new(){}fn into_inner(){}fn main(){}")
                .unwrap();
        for (index, atom) in [
            (0, BuiltinAtom::CloneMethod),
            (1, BuiltinAtom::CloneFromMethod),
            (2, BuiltinAtom::DropMethod),
            (3, BuiltinAtom::ManuallyDropNew),
            (4, BuiltinAtom::ManuallyDropIntoInner),
            (2, BuiltinAtom::ManuallyDropDrop),
        ]
        .into_iter()
        {
            let ident = program.functions[index].name.segments()[0];
            let member = program.symbols.key(program.names.as_ref(), atom).unwrap();
            assert_eq!(member.class(), LookupClass::Member);
            assert!(
                program
                    .symbols
                    .matches(program.names.as_ref(), ident, atom)
                    .unwrap()
            );
            assert_ne!(
                member,
                program
                    .names
                    .key(ident, LookupClass::Item(Namespace::Value))
                    .unwrap()
            );
        }
        let foreign = parse("fn clone(){}fn main(){}").unwrap();
        assert!(
            program
                .symbols
                .key(foreign.names.as_ref(), BuiltinAtom::CloneMethod)
                .unwrap_err()
                .starts_with("contract:")
        );
        assert!(
            program
                .symbols
                .matches(
                    program.names.as_ref(),
                    foreign.functions[0].name.segments()[0],
                    BuiltinAtom::CloneMethod
                )
                .unwrap_err()
                .starts_with("contract:")
        );
    }
}
