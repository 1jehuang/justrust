//! Checked source-arm alternatives. Locals belong to one source arm, not each row.
use super::*;

pub(super) fn validate_or_rows(rows: &[MatchPattern]) -> Result<(), String> {
    if !(2..=MAX_MATCH_ARMS).contains(&rows.len()) {
        return Err("grouped or-pattern row count must be between 2 and 4096".into());
    }
    if rows
        .iter()
        .any(|row| matches!(row, MatchPattern::Or { .. }))
    {
        return Err("grouped or-pattern rows must be flattened".into());
    }
    Ok(())
}

impl CheckedFunction {
    pub(super) fn validate_or_pattern(
        &self,
        ty: &Ty,
        rows: &[MatchPattern],
        parameters: &std::collections::HashSet<LocalId>,
        previous: &mut std::collections::HashSet<LocalId>,
        facts: FactsCtx<'_>,
    ) -> Result<(), String> {
        validate_or_rows(rows)?;
        let mut canonical = None;
        for row in rows {
            let mut bindings = std::collections::HashSet::new();
            self.validate_pattern(ty, row, parameters, &mut bindings, facts)?;
            if let Some(expected) = &canonical {
                if expected != &bindings {
                    return Err(
                        "or-pattern rows must initialize the same canonical bindings".into(),
                    );
                }
            } else {
                canonical = Some(bindings);
            }
        }
        for binding in canonical.unwrap_or_default() {
            if !previous.insert(binding) {
                return Err("match bindings must be distinct fresh locals".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (CheckedFunction, ExprId, Vec<TypedMatchArm>) {
        let program = crate::compiler::frontend::parse(
            "fn main(){let _v=match (true,false){(true,x)|(x,false) if x=>1,_=>0};}",
        )
        .unwrap();
        let signatures = crate::compiler::frontend::signature_table(&program).unwrap();
        let checked =
            crate::compiler::frontend::type_function_with_signatures(&program, 0, &signatures)
                .unwrap();
        let (scrutinee, arms) = checked
            .exprs
            .iter()
            .find_map(|expr| match &expr.kind {
                ExprKind::Match {
                    scrutinee, arms, ..
                } => Some((*scrutinee, arms.clone())),
                _ => None,
            })
            .unwrap();
        (checked, scrutinee, arms)
    }

    #[test]
    fn grouped_rows_share_bindings_but_independent_arms_cannot() {
        let (checked, scrutinee, mut arms) = fixture();
        let parameters = checked.params.iter().copied().collect();
        checked
            .validate_match(scrutinee, &arms, &parameters)
            .unwrap();
        arms.push(arms[0].clone());
        assert!(
            checked
                .validate_match(scrutinee, &arms, &parameters)
                .unwrap_err()
                .contains("distinct fresh locals")
        );
    }

    #[test]
    fn grouped_rows_require_equal_binding_sets_and_flat_bounded_rows() {
        let (checked, scrutinee, mut arms) = fixture();
        let parameters = checked.params.iter().copied().collect();
        let MatchPattern::Or { rows, .. } = &mut arms[0].pattern else {
            panic!("missing source group")
        };
        rows[1] = MatchPattern::Wildcard;
        assert!(
            checked
                .validate_match(scrutinee, &arms, &parameters)
                .unwrap_err()
                .contains("same canonical bindings")
        );
        let nested = MatchPattern::Or {
            ty: Ty::Bool,
            rows: vec![MatchPattern::Wildcard, MatchPattern::Wildcard],
        };
        assert!(
            validate_or_rows(&[nested, MatchPattern::Wildcard])
                .unwrap_err()
                .contains("flattened")
        );
        assert!(validate_or_rows(&[MatchPattern::Wildcard]).is_err());
    }
}
