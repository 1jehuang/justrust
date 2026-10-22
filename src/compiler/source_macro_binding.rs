//! Source-owned macro routing. Mounted modules do not merge crate lexical scope.
//! This is a pure query over the checked token snapshot, not a name-based builtin
//! exception or permission derived from a printable path.
use super::expansion_span::SpanId;
use super::macro_rules::DefinitionSite;
use super::token_session::{SourceFileHandle, TokenError, TokenSession};

pub(super) fn definition_root(
    session: &TokenSession,
    span: SpanId,
) -> Result<SourceFileHandle, TokenError> {
    let original = session.original(span, 512)?;
    let source = session.sources().render_span(original)?;
    let file = session.sources().file(source.file.0 as usize)?;
    session.sources().source_crate_root(file)
}

/// Macro-name lookup uses the invocation environment. For generated calls this
/// is the admitted expansion's call chain, not the transcriber's token origin.
pub(super) fn invocation_root(
    session: &TokenSession,
    mut span: SpanId,
) -> Result<SourceFileHandle, TokenError> {
    for _ in 0..512 {
        session
            .budget()
            .charge(super::expansion_budget::Counter::Work, 1)?;
        let occurrence = session.span(span)?;
        match occurrence.expansion {
            Some(expansion) => span = session.expansion(expansion)?.call,
            None => return definition_root(session, span),
        }
    }
    Err(TokenError::TraceTruncated)
}

pub(super) fn validate_definition(
    session: &TokenSession,
    site: DefinitionSite,
) -> Result<SourceFileHandle, TokenError> {
    let root = definition_root(session, site.span)?;
    if session.crate_root(site.defining_crate)? != root {
        return Err(TokenError::InvalidHandle);
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;
    use crate::compiler::expansion_span::ExpansionRecord;
    use crate::compiler::source::{FileId, SourceMap};
    use crate::compiler::token::Edition;
    use crate::compiler::token_session::SourceSnapshot;
    use std::sync::Arc;

    fn sources() -> SourceMap {
        let mut map = SourceMap::default();
        // Deliberately identical paths and text: neither is binding authority.
        for _ in 0..3 {
            map.add("same.rs".into(), Arc::from("name"));
        }
        map
    }

    #[test]
    fn source_roots_are_sealed_owner_checked_and_not_path_or_file_identity() {
        let budget = ExpansionBudget::default();
        let snapshot = SourceSnapshot::copy_with_crates(
            &sources(),
            &[FileId(0), FileId(0), FileId(2)],
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let sibling = snapshot.file(1).unwrap();
        assert_eq!(
            snapshot.source_crate_root(sibling).unwrap(),
            snapshot.file(0).unwrap()
        );
        let other = SourceSnapshot::copy_borrowed(
            &sources(),
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            other.source_crate_root(sibling),
            Err(TokenError::ForeignHandle)
        ));
        let (mut session, _) = TokenSession::standalone(snapshot.clone(), budget).unwrap();
        let definition = session
            .original_span(snapshot.span(sibling, 0, 4).unwrap())
            .unwrap();
        let root = session
            .register_crate_root(snapshot.file(0).unwrap())
            .unwrap();
        let wrong = session
            .register_crate_root(snapshot.file(2).unwrap())
            .unwrap();
        let site = DefinitionSite {
            span: definition,
            edition: Edition::E2024,
            defining_crate: root,
        };
        assert!(validate_definition(&session, site).is_ok());
        assert!(matches!(
            validate_definition(
                &session,
                DefinitionSite {
                    defining_crate: wrong,
                    ..site
                }
            ),
            Err(TokenError::InvalidHandle)
        ));
        let file_not_root = session.register_crate_root(sibling).unwrap();
        assert!(matches!(
            validate_definition(
                &session,
                DefinitionSite {
                    defining_crate: file_not_root,
                    ..site
                }
            ),
            Err(TokenError::InvalidHandle)
        ));
    }

    #[test]
    fn generated_macro_names_route_by_call_not_transcriber_source() {
        let budget = ExpansionBudget::default();
        let snapshot = SourceSnapshot::copy_with_crates(
            &sources(),
            &[FileId(0), FileId(0), FileId(2)],
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let (mut session, _) = TokenSession::standalone(snapshot.clone(), budget).unwrap();
        let definition = session
            .original_span(snapshot.span(snapshot.file(2).unwrap(), 0, 4).unwrap())
            .unwrap();
        let call = session
            .original_span(snapshot.span(snapshot.file(1).unwrap(), 0, 4).unwrap())
            .unwrap();
        let defining_crate = session
            .register_crate_root(snapshot.file(2).unwrap())
            .unwrap();
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition,
                call,
                parent: None,
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let token = session
            .with_context(definition, session.root_context(), Some(expansion))
            .unwrap();
        assert_eq!(
            definition_root(&session, token).unwrap(),
            snapshot.file(2).unwrap()
        );
        assert_eq!(
            invocation_root(&session, token).unwrap(),
            snapshot.file(0).unwrap()
        );
    }

    #[test]
    fn malformed_crate_ownership_never_defaults_to_user_root() {
        let budget = ExpansionBudget::default();
        for roots in [
            vec![FileId(0)],
            vec![FileId(0), FileId(2), FileId(2)],
            vec![FileId(1), FileId(1), FileId(2)],
        ] {
            assert!(
                SourceSnapshot::copy_with_crates(
                    &sources(),
                    &roots,
                    &budget,
                    crate::compiler::owner_path::standalone_issued(
                        &budget,
                        crate::compiler::owner_path::OwnerDomain::SourceSnapshot
                    )
                    .unwrap()
                )
                .is_err()
            );
        }
    }
}
