//! Explicit compiler-derived identifiers, authenticated by an original derive.
//! No caller-supplied spelling and no claim that the member was an original token.
use super::*;
use crate::compiler::expansion_span::{SpanOrigin, SyntheticReason};
use crate::compiler::semantic_scope::{DerivedMemberAdmission, DerivedMemberKind, ScopeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivedAtom {
    Method,
    Receiver,
    FormatterParameter,
    FormatterWriteStr,
}
impl DerivedAtom {
    fn spelling(self, kind: DerivedMemberKind) -> Result<&'static str, ScopeError> {
        Ok(match (kind, self) {
            (DerivedMemberKind::Clone, Self::Method) => "clone",
            (DerivedMemberKind::Debug, Self::Method) => "fmt",
            (_, Self::Receiver) => "self",
            (DerivedMemberKind::Debug, Self::FormatterParameter) => "__jr_derived_formatter",
            (DerivedMemberKind::Debug, Self::FormatterWriteStr) => "write_str",
            _ => return Err(ScopeError::ConflictingAssociation),
        })
    }
}
impl NameStoreBuilder {
    pub(crate) fn derived_identifier(
        &mut self,
        admission: &DerivedMemberAdmission,
        atom: DerivedAtom,
    ) -> Result<AstIdent, ScopeError> {
        let anchor = admission.anchor(self)?;
        let text = atom.spelling(admission.kind())?;
        let spelling = self.store.session.intern_symbol(text)?;
        let span = self
            .store
            .session
            .synthetic_span(anchor, SyntheticReason::DerivedMember)?;
        let context = self.store.session.root_context();
        let expansion = self.store.session.span(span)?.expansion;
        let span = self.store.session.with_context(span, context, expansion)?;
        Ok(self.append(NameData {
            atom: NameAtomId(spelling),
            spelling,
            context,
            span,
            raw: false,
            edition: self.edition(),
            anonymous_closure: false,
        })?)
    }
}

pub(crate) fn validate_derived_identifier(
    names: &impl crate::compiler::semantic_path::NameAccess,
    admission: &DerivedMemberAdmission,
    atom: DerivedAtom,
    ident: AstIdent,
) -> Result<(), ScopeError> {
    let anchor = admission.anchor(names)?;
    names.check(ident)?;
    let span = names.session().span(names.span(ident)?)?;
    if names.spelling(ident)? != atom.spelling(admission.kind())?
        || span.origin
            != (SpanOrigin::Synthetic {
                anchor,
                reason: SyntheticReason::DerivedMember,
            })
        || !matches!(
            names.session().context(span.context)?,
            crate::compiler::hygiene::ContextData::Root
        )
        || span.expansion != names.session().span(anchor)?.expansion
    {
        return Err(ScopeError::ConflictingAssociation);
    }
    Ok(())
}
