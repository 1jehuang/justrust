//! Nested or-patterns. `Some(1 | 2)` and `(A | B, x)` are flattened into the
//! equivalent ordered root alternatives (`Some(1) | Some(2)`), which the match
//! checker then checks as one arm per row sharing the source body and guard.
//! Row order is left-to-right, matching Rust's first-match semantics.
use super::*;

/// Upper bound on rows produced by flattening one source pattern.
const MAX_FLAT_ROWS: usize = 4096;

pub(super) fn has_nested_alternatives(pattern: &Pattern, root: bool) -> bool {
    match pattern {
        Pattern::Alternatives(rows) => {
            !root || rows.iter().any(|row| has_nested_alternatives(row, false))
        }
        Pattern::Apply { pattern, .. }
        | Pattern::Ref { pattern, .. }
        | Pattern::At { pattern, .. } => has_nested_alternatives(pattern, false),
        Pattern::Variant { fields, .. } => fields
            .iter()
            .any(|(_, p)| has_nested_alternatives(p, false)),
        Pattern::Struct { fields, .. } => fields
            .iter()
            .any(|(_, p)| has_nested_alternatives(p, false)),
        Pattern::Tuple(fields) | Pattern::Array(fields) => {
            fields.iter().any(|p| has_nested_alternatives(p, false))
        }
        Pattern::TupleRest(rest)
        | Pattern::ArrayRest(rest)
        | Pattern::VariantRest { fields: rest, .. } => rest
            .prefix
            .iter()
            .chain(&rest.suffix)
            .any(|p| has_nested_alternatives(p, false)),
        _ => false,
    }
}

fn product<T: Clone>(parts: Vec<Vec<T>>, budget: &ir::PlanBudget) -> Result<Vec<Vec<T>>, String> {
    let mut rows: Vec<Vec<T>> = vec![Vec::new()];
    for options in parts {
        let next = rows
            .len()
            .checked_mul(options.len())
            .filter(|n| *n <= MAX_FLAT_ROWS)
            .ok_or("or-pattern expansion exceeds 4096 alternatives")?;
        budget.charge_work(next)?;
        let mut out = Vec::with_capacity(next);
        for row in &rows {
            for option in &options {
                let mut row = row.clone();
                row.push(option.clone());
                out.push(row);
            }
        }
        rows = out;
    }
    Ok(rows)
}

fn flat_rest(
    rest: &PositionalRest,
    budget: &ir::PlanBudget,
) -> Result<Vec<PositionalRest>, String> {
    let parts = rest
        .prefix
        .iter()
        .chain(&rest.suffix)
        .map(|p| flatten(p, budget))
        .collect::<Result<Vec<_>, _>>()?;
    let split = rest.prefix.len();
    Ok(product(parts, budget)?
        .into_iter()
        .map(|mut row| {
            let suffix = row.split_off(split);
            PositionalRest {
                prefix: row,
                suffix,
            }
        })
        .collect())
}

/// Every alternative-free pattern equivalent to `pattern`, in match order.
pub(super) fn flatten(pattern: &Pattern, budget: &ir::PlanBudget) -> Result<Vec<Pattern>, String> {
    budget.charge_work(1)?;
    Ok(match pattern {
        Pattern::Alternatives(rows) => {
            let mut out = Vec::new();
            for row in rows {
                out.extend(flatten(row, budget)?);
                if out.len() > MAX_FLAT_ROWS {
                    return Err("or-pattern expansion exceeds 4096 alternatives".into());
                }
            }
            out
        }
        Pattern::Apply { path, pattern } => flatten(pattern, budget)?
            .into_iter()
            .map(|pattern| Pattern::Apply {
                path: path.clone(),
                pattern: Box::new(pattern),
            })
            .collect(),
        Pattern::Ref { kind, pattern } => flatten(pattern, budget)?
            .into_iter()
            .map(|pattern| Pattern::Ref {
                kind: *kind,
                pattern: Box::new(pattern),
            })
            .collect(),
        Pattern::At {
            name,
            mutable,
            by_ref,
            pattern,
        } => flatten(pattern, budget)?
            .into_iter()
            .map(|pattern| Pattern::At {
                name: *name,
                mutable: *mutable,
                by_ref: *by_ref,
                pattern: Box::new(pattern),
            })
            .collect(),
        Pattern::Variant {
            name,
            shape,
            fields,
            rest,
        } => {
            let parts = fields
                .iter()
                .map(|(_, p)| flatten(p, budget))
                .collect::<Result<Vec<_>, _>>()?;
            product(parts, budget)?
                .into_iter()
                .map(|row| Pattern::Variant {
                    name: name.clone(),
                    shape: *shape,
                    fields: fields.iter().map(|(s, _)| s.clone()).zip(row).collect(),
                    rest: *rest,
                })
                .collect()
        }
        Pattern::Struct { name, fields, rest } => {
            let parts = fields
                .iter()
                .map(|(_, p)| flatten(p, budget))
                .collect::<Result<Vec<_>, _>>()?;
            product(parts, budget)?
                .into_iter()
                .map(|row| Pattern::Struct {
                    name: name.clone(),
                    fields: fields.iter().map(|(s, _)| *s).zip(row).collect(),
                    rest: *rest,
                })
                .collect()
        }
        Pattern::Tuple(fields) => {
            let parts = fields
                .iter()
                .map(|p| flatten(p, budget))
                .collect::<Result<Vec<_>, _>>()?;
            product(parts, budget)?
                .into_iter()
                .map(Pattern::Tuple)
                .collect()
        }
        Pattern::Array(fields) => {
            let parts = fields
                .iter()
                .map(|p| flatten(p, budget))
                .collect::<Result<Vec<_>, _>>()?;
            product(parts, budget)?
                .into_iter()
                .map(Pattern::Array)
                .collect()
        }
        Pattern::TupleRest(rest) => flat_rest(rest, budget)?
            .into_iter()
            .map(Pattern::TupleRest)
            .collect(),
        Pattern::ArrayRest(rest) => flat_rest(rest, budget)?
            .into_iter()
            .map(Pattern::ArrayRest)
            .collect(),
        Pattern::VariantRest { name, fields } => flat_rest(fields, budget)?
            .into_iter()
            .map(|fields| Pattern::VariantRest {
                name: name.clone(),
                fields,
            })
            .collect(),
        other => vec![other.clone()],
    })
}
