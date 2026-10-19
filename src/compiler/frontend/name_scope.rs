//! Checked semantic comparisons for source occurrences. Display text never
//! participates in equality, binder lookup, or path authority.
use super::*;
use crate::compiler::expansion_budget::{Counter, Storage};
use crate::compiler::token_session::{BudgetVec, TokenError};

fn work(names: &impl NameAccess, amount: usize) -> Result<(), String> {
    names
        .budget()
        .charge(Counter::Work, amount as u64)
        .map_err(|error| {
            super::super::source::SourceError::from_token(TokenError::Budget(error)).to_string()
        })
}

pub(super) fn ident_eq(
    names: &impl NameAccess,
    left: AstIdent,
    right: AstIdent,
    class: LookupClass,
) -> Result<bool, String> {
    let left = names
        .key(left, class)
        .map_err(native_adapter::name_diagnostic)?;
    let right = names
        .key(right, class)
        .map_err(native_adapter::name_diagnostic)?;
    Ok(left == right)
}

pub(super) fn path_key(
    names: &impl NameAccess,
    path: &AstPath,
    namespace: Namespace,
) -> Result<PathKey, String> {
    path.check(names).map_err(native_adapter::name_diagnostic)?;
    work(names, path.len().saturating_add(1))?;
    Ok(path.key(namespace))
}

pub(super) fn region_eq(
    names: &impl NameAccess,
    left: RegionName,
    right: RegionName,
) -> Result<bool, String> {
    work(names, 1)?;
    match (left, right) {
        (RegionName::Named(a), RegionName::Named(b)) => {
            ident_eq(names, a, b, LookupClass::LifetimeParameter)
        }
        (RegionName::Static, RegionName::Static) | (RegionName::Infer, RegionName::Infer) => {
            Ok(true)
        }
        (RegionName::Named(name), _) | (_, RegionName::Named(name)) => {
            names.check(name).map_err(native_adapter::name_diagnostic)?;
            Ok(false)
        }
        _ => Ok(false),
    }
}

impl FieldSelector {
    pub fn semantic_eq(&self, other: &Self, names: &impl NameAccess) -> Result<bool, String> {
        work(names, 1)?;
        match (self, other) {
            (Self::Named(a), Self::Named(b)) => ident_eq(names, *a, *b, LookupClass::Member),
            (Self::Named(a), _) | (_, Self::Named(a)) => {
                names.check(*a).map_err(native_adapter::name_diagnostic)?;
                Ok(false)
            }
            (Self::Deref, Self::Deref) => Ok(true),
            (Self::ConstantIndex(a), Self::ConstantIndex(b)) | (Self::Tuple(a), Self::Tuple(b)) => {
                Ok(a == b)
            }
            _ => Ok(false),
        }
    }
}

fn transport(error: TokenError) -> String {
    super::super::source::SourceError::from_token(error).to_string()
}

/// Checks each side independently before comparison can discard unequal or
/// unzipped branches. The stack is flat, fallible and charged before growth.
fn validate_type_names(ty: &Type, names: &impl NameAccess) -> Result<(), String> {
    let mut pending = BudgetVec::new(names.budget(), Storage::Temporary).map_err(transport)?;
    work(names, 1)?;
    pending.push(ty).map_err(transport)?;
    while let Some(ty) = pending.pop() {
        work(names, 1)?;
        match ty {
            Type::Projection(projection) => {
                let item = match projection.as_ref() {
                    SourceProjection::Qualified {
                        self_ty,
                        trait_path,
                        item,
                    } => {
                        work(names, 1)?;
                        pending.push(self_ty).map_err(transport)?;
                        if let TraitPath::Source(path) = trait_path {
                            path.path(names)?;
                            work(names, path.segments.len())?;
                            for segment in &path.segments {
                                names
                                    .key(segment.name, LookupClass::Item(Namespace::Type))
                                    .map_err(native_adapter::name_diagnostic)?;
                                work(names, segment.args.len())?;
                                for binding in &segment.associated_types {
                                    work(names, 1)?;
                                    names
                                        .key(binding.name, LookupClass::Member)
                                        .map_err(native_adapter::name_diagnostic)?;
                                }
                                for actual in segment.args.iter().chain(
                                    segment
                                        .associated_types
                                        .iter()
                                        .map(|binding| &binding.value),
                                ) {
                                    pending.push(actual).map_err(transport)?;
                                }
                            }
                        }
                        item
                    }
                    SourceProjection::SelfItem { item } => item,
                };
                names
                    .key(*item, LookupClass::Member)
                    .map_err(native_adapter::name_diagnostic)?;
            }
            Type::Param(name) => {
                names
                    .key(*name, LookupClass::TypeParameter)
                    .map_err(native_adapter::name_diagnostic)?;
            }
            Type::Named(path) => {
                path_key(names, path, Namespace::Type)?;
            }
            Type::Ref {
                region, referent, ..
            } => {
                if let Some(RegionName::Named(name)) = region {
                    names
                        .key(*name, LookupClass::LifetimeParameter)
                        .map_err(native_adapter::name_diagnostic)?;
                }
                work(names, 1)?;
                pending.push(referent.as_ref()).map_err(transport)?;
            }
            Type::Dyn(trait_path) => {
                if let TraitPath::Source(path) = trait_path.as_ref() {
                    path.path(names)?;
                    work(names, path.segments.len())?;
                    for segment in &path.segments {
                        names
                            .key(segment.name, LookupClass::Item(Namespace::Type))
                            .map_err(native_adapter::name_diagnostic)?;
                    }
                }
            }
            Type::ParamArray(element, name) => {
                names
                    .key(*name, LookupClass::TypeParameter)
                    .map_err(native_adapter::name_diagnostic)?;
                work(names, 1)?;
                pending.push(element.as_ref()).map_err(transport)?;
            }
            Type::ConstArg(_) => {}
            Type::Array(element, _)
            | Type::Slice(element)
            | Type::RawPtr {
                pointee: element, ..
            } => {
                work(names, 1)?;
                pending.push(element.as_ref()).map_err(transport)?;
            }
            Type::Tuple(elements) => {
                work(names, elements.len())?;
                for element in elements {
                    pending.push(element).map_err(transport)?;
                }
            }
            Type::Apply(path) => {
                work(names, path.segments.len())?;
                for segment in &path.segments {
                    names
                        .key(segment.name, LookupClass::Item(Namespace::Type))
                        .map_err(native_adapter::name_diagnostic)?;
                    work(names, segment.args.len())?;
                    for binding in &segment.associated_types {
                        work(names, 1)?;
                        names
                            .key(binding.name, LookupClass::Member)
                            .map_err(native_adapter::name_diagnostic)?;
                    }
                    for argument in segment.args.iter().chain(
                        segment
                            .associated_types
                            .iter()
                            .map(|binding| &binding.value),
                    ) {
                        pending.push(argument).map_err(transport)?;
                    }
                }
            }
            Type::SelfType
            | Type::Int(_)
            | Type::Float(_)
            | Type::Char
            | Type::Str
            | Type::Bool
            | Type::Unit
            | Type::Never
            | Type::Infer => {}
        }
    }
    Ok(())
}

impl Type {
    /// Compares semantic keys, not source occurrence identity. Both sides are
    /// checked in full even if an earlier structural difference is observed.
    pub fn semantic_eq(&self, other: &Self, names: &impl NameAccess) -> Result<bool, String> {
        validate_type_names(self, names)?;
        validate_type_names(other, names)?;
        let mut pending = BudgetVec::new(names.budget(), Storage::Temporary)
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        pending
            .push((self, other))
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        let mut equal = true;
        while let Some((left, right)) = pending.pop() {
            work(names, 1)?;
            match (left, right) {
                (Type::Projection(a), Type::Projection(b)) => match (a.as_ref(), b.as_ref()) {
                    (
                        SourceProjection::Qualified {
                            self_ty: a,
                            trait_path: at,
                            item: ai,
                        },
                        SourceProjection::Qualified {
                            self_ty: b,
                            trait_path: bt,
                            item: bi,
                        },
                    ) => {
                        equal &= ident_eq(names, *ai, *bi, LookupClass::Member)?;
                        work(names, 1)?;
                        pending.push((a, b)).map_err(transport)?;
                        match (at, bt) {
                            (TraitPath::Builtin(a), TraitPath::Builtin(b)) => equal &= a == b,
                            (TraitPath::Source(a), TraitPath::Source(b)) => {
                                equal &=
                                    a.anchor == b.anchor && a.segments.len() == b.segments.len();
                                for (a, b) in a.segments.iter().zip(&b.segments) {
                                    equal &= ident_eq(
                                        names,
                                        a.name,
                                        b.name,
                                        LookupClass::Item(Namespace::Type),
                                    )?;
                                    equal &= a.args.len() == b.args.len();
                                    equal &= a.associated_types.len() == b.associated_types.len();
                                    for (a, b) in a.associated_types.iter().zip(&b.associated_types)
                                    {
                                        equal &=
                                            ident_eq(names, a.name, b.name, LookupClass::Member)?;
                                        work(names, 1)?;
                                        pending.push((&a.value, &b.value)).map_err(transport)?;
                                    }
                                    for (a, b) in a.args.iter().zip(&b.args) {
                                        work(names, 1)?;
                                        pending.push((a, b)).map_err(transport)?;
                                    }
                                }
                            }
                            _ => equal = false,
                        }
                    }
                    (
                        SourceProjection::SelfItem { item: a },
                        SourceProjection::SelfItem { item: b },
                    ) => {
                        equal &= ident_eq(names, *a, *b, LookupClass::Member)?;
                    }
                    _ => equal = false,
                },
                (Type::Param(a), Type::Param(b)) => {
                    equal &= ident_eq(names, *a, *b, LookupClass::TypeParameter)?
                }
                (Type::ConstArg(a), Type::ConstArg(b)) => equal &= a == b,
                (Type::ParamArray(a, an), Type::ParamArray(b, bn)) => {
                    equal &= ident_eq(names, *an, *bn, LookupClass::TypeParameter)?;
                    work(names, 1)?;
                    pending.push((a.as_ref(), b.as_ref())).map_err(|e| {
                        super::super::source::SourceError::from_token(e).to_string()
                    })?;
                }
                (Type::Named(a), Type::Named(b)) => {
                    equal &=
                        path_key(names, a, Namespace::Type)? == path_key(names, b, Namespace::Type)?
                }
                (
                    Type::Ref {
                        kind: ak,
                        region: ar,
                        referent: a,
                    },
                    Type::Ref {
                        kind: bk,
                        region: br,
                        referent: b,
                    },
                ) => {
                    equal &= ak == bk;
                    equal &= match (*ar, *br) {
                        (Some(a), Some(b)) => region_eq(names, a, b)?,
                        (None, None) => true,
                        (Some(RegionName::Named(a)), None) | (None, Some(RegionName::Named(a))) => {
                            names.check(a).map_err(native_adapter::name_diagnostic)?;
                            false
                        }
                        _ => false,
                    };
                    work(names, 1)?;
                    pending.push((a.as_ref(), b.as_ref())).map_err(|e| {
                        super::super::source::SourceError::from_token(e).to_string()
                    })?;
                }
                (Type::Array(a, an), Type::Array(b, bn)) => {
                    equal &= an == bn;
                    work(names, 1)?;
                    pending.push((a.as_ref(), b.as_ref())).map_err(|e| {
                        super::super::source::SourceError::from_token(e).to_string()
                    })?;
                }
                (Type::Slice(a), Type::Slice(b)) => {
                    work(names, 1)?;
                    pending.push((a.as_ref(), b.as_ref())).map_err(|e| {
                        super::super::source::SourceError::from_token(e).to_string()
                    })?;
                }
                (
                    Type::RawPtr {
                        kind: ak,
                        pointee: a,
                    },
                    Type::RawPtr {
                        kind: bk,
                        pointee: b,
                    },
                ) => {
                    equal &= ak == bk;
                    work(names, 1)?;
                    pending.push((a.as_ref(), b.as_ref())).map_err(|e| {
                        super::super::source::SourceError::from_token(e).to_string()
                    })?;
                }
                (Type::Dyn(a), Type::Dyn(b)) => match (a.as_ref(), b.as_ref()) {
                    (TraitPath::Builtin(a), TraitPath::Builtin(b)) => equal &= a == b,
                    (TraitPath::Source(a), TraitPath::Source(b)) => {
                        // Written trait object principals are argument-free.
                        let a = a.path(names)?;
                        let b = b.path(names)?;
                        equal &= path_key(names, &a, Namespace::Type)?
                            == path_key(names, &b, Namespace::Type)?;
                    }
                    _ => equal = false,
                },
                (Type::Tuple(a), Type::Tuple(b)) => {
                    equal &= a.len() == b.len();
                    for (a, b) in a.iter().zip(b) {
                        work(names, 1)?;
                        pending.push((a, b)).map_err(|e| {
                            super::super::source::SourceError::from_token(e).to_string()
                        })?;
                    }
                }
                (Type::Apply(a), Type::Apply(b)) => {
                    equal &= a.anchor == b.anchor && a.segments.len() == b.segments.len();
                    for (a, b) in a.segments.iter().zip(&b.segments) {
                        equal &=
                            ident_eq(names, a.name, b.name, LookupClass::Item(Namespace::Type))?;
                        equal &= a.args.len() == b.args.len();
                        equal &= a.associated_types.len() == b.associated_types.len();
                        for (a, b) in a.associated_types.iter().zip(&b.associated_types) {
                            equal &= ident_eq(names, a.name, b.name, LookupClass::Member)?;
                            work(names, 1)?;
                            pending.push((&a.value, &b.value)).map_err(transport)?;
                        }
                        for (a, b) in a.args.iter().zip(&b.args) {
                            work(names, 1)?;
                            pending.push((a, b)).map_err(|e| {
                                super::super::source::SourceError::from_token(e).to_string()
                            })?;
                        }
                    }
                }
                (Type::Int(a), Type::Int(b)) => equal &= a == b,
                (Type::Float(a), Type::Float(b)) => equal &= a == b,
                (Type::SelfType, Type::SelfType)
                | (Type::Char, Type::Char)
                | (Type::Str, Type::Str)
                | (Type::Bool, Type::Bool)
                | (Type::Unit, Type::Unit)
                | (Type::Never, Type::Never)
                | (Type::Infer, Type::Infer) => (),
                _ => {
                    // Both entire inputs were checked before structural
                    // comparison, including descendants of unequal variants.
                    equal = false;
                }
            }
        }
        Ok(equal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::{BudgetFailureKind, BudgetResource, ExpansionBudget};
    use crate::compiler::semantic_name::NameError;
    use crate::compiler::source::PreparedLexInput;
    use crate::compiler::token::{Edition, InputToken};
    use crate::compiler::token_session::TokenSession;
    use std::path::Path;

    fn names(text: &str, budget: &ExpansionBudget) -> (Arc<NameStore>, Vec<AstIdent>) {
        let input =
            PreparedLexInput::identity(Path::new("comparison-control.rs"), text, budget).unwrap();
        let (mut session, session_cap) =
            TokenSession::standalone(input.snapshot().clone(), budget.clone()).unwrap();
        let tokens =
            crate::compiler::lexer::lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let identifiers = tokens
            .events()
            .iter()
            .filter_map(|event| {
                let InputToken::Leaf(token) = event.token else {
                    return None;
                };
                Some(builder.from_token(token, Edition::E2024).unwrap())
            })
            .collect();
        (builder.freeze().unwrap(), identifiers)
    }

    fn apply(name: AstIdent, arguments: Vec<Type>) -> Type {
        Type::Apply(GenericPath {
            resolved_use: None,
            anchor: PathAnchor::Relative,
            segments: vec![PathSegment {
                name,
                args: arguments,
                associated_types: Vec::new(),
            }],
        })
    }

    #[test]
    fn projections_compare_all_semantic_children_but_do_not_conflate_source_forms() {
        let budget = ExpansionBudget::default();
        let (names, ids) = names("Trait Trait Out Out T U", &budget);
        let projection = |trait_name, item, argument| {
            Type::Projection(Box::new(SourceProjection::Qualified {
                self_ty: Type::Bool,
                trait_path: TraitPath::Source(GenericPath {
                    anchor: PathAnchor::Relative,
                    segments: vec![PathSegment {
                        name: trait_name,
                        args: vec![argument],
                        associated_types: Vec::new(),
                    }],
                    resolved_use: None,
                }),
                item,
            }))
        };
        let a = projection(ids[0], ids[2], Type::Param(ids[4]));
        let b = projection(ids[1], ids[3], Type::Param(ids[4]));
        assert!(a.semantic_eq(&b, names.as_ref()).unwrap());
        assert!(
            !a.semantic_eq(
                &projection(ids[1], ids[3], Type::Param(ids[5])),
                names.as_ref()
            )
            .unwrap()
        );
        assert!(
            !a.semantic_eq(
                &Type::Projection(Box::new(SourceProjection::SelfItem { item: ids[2] })),
                names.as_ref()
            )
            .unwrap()
        );
        let (_, foreign) = self::names("Out", &ExpansionBudget::default());
        assert!(
            Type::Unit
                .semantic_eq(&projection(ids[0], foreign[0], Type::Bool), names.as_ref())
                .is_err()
        );
        assert!(
            Type::Unit
                .semantic_eq(
                    &projection(ids[0], ids[2], Type::Param(foreign[0])),
                    names.as_ref()
                )
                .is_err()
        );
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn equal_names_ignore_occurrence_spans_but_not_structure() {
        let budget = ExpansionBudget::default();
        let (names, ids) = names("T T U", &budget);
        assert_ne!(names.span(ids[0]).unwrap(), names.span(ids[1]).unwrap());
        assert!(
            Type::Param(ids[0])
                .semantic_eq(&Type::Param(ids[1]), names.as_ref())
                .unwrap()
        );
        assert!(
            !Type::Param(ids[0])
                .semantic_eq(&Type::Param(ids[2]), names.as_ref())
                .unwrap()
        );
        let left = apply(ids[0], vec![Type::Param(ids[0])]);
        let right = apply(ids[1], vec![Type::Param(ids[1])]);
        assert!(left.semantic_eq(&right, names.as_ref()).unwrap());
        assert!(
            !left
                .semantic_eq(&apply(ids[1], vec![]), names.as_ref())
                .unwrap()
        );
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn foreign_names_in_unequal_variants_and_unzipped_tails_always_reject() {
        let budget = ExpansionBudget::default();
        let (names, own) = names("T", &budget);
        let (foreign_names, foreign) = self::names("T", &ExpansionBudget::default());
        let foreign_name = foreign[0];
        let foreign_path = AstPath::new(
            foreign_names.as_ref(),
            PathAnchor::Relative,
            &[foreign_name],
        )
        .unwrap();
        let i32_type = Type::Int(ir::IntTy::I32);
        let cases = vec![
            (Type::Bool, Type::Param(foreign_name)),
            (Type::Bool, Type::Named(foreign_path)),
            (
                Type::Tuple(vec![i32_type.clone()]),
                Type::Tuple(vec![i32_type.clone(), Type::Param(foreign_name)]),
            ),
            (
                apply(own[0], vec![]),
                apply(own[0], vec![Type::Param(foreign_name)]),
            ),
            (
                apply(own[0], vec![]),
                Type::Apply(GenericPath {
                    resolved_use: None,
                    anchor: PathAnchor::Relative,
                    segments: vec![
                        PathSegment {
                            name: own[0],
                            args: vec![],
                            associated_types: Vec::new(),
                        },
                        PathSegment {
                            name: foreign_name,
                            args: vec![],
                            associated_types: Vec::new(),
                        },
                    ],
                }),
            ),
            (
                Type::Bool,
                Type::Ref {
                    kind: ir::BorrowKind::Shared,
                    region: Some(RegionName::Named(foreign_name)),
                    referent: Box::new(i32_type.clone()),
                },
            ),
            (
                Type::Bool,
                Type::Array(Box::new(Type::Param(foreign_name)), 1),
            ),
        ];
        let expected = native_adapter::name_diagnostic(NameError::Token(TokenError::ForeignHandle));
        let retained = budget.snapshot().retained_bytes;
        for (left, right) in cases {
            assert_eq!(
                left.semantic_eq(&right, names.as_ref()).unwrap_err(),
                expected
            );
            assert_eq!(
                right.semantic_eq(&left, names.as_ref()).unwrap_err(),
                expected
            );
            let snapshot = budget.snapshot();
            assert_eq!(snapshot.temporary_bytes, 0);
            assert_eq!(snapshot.retained_bytes, retained);
        }
    }

    #[test]
    fn denied_bulk_scan_releases_temporary_storage_and_latches_exact_limit() {
        let budget = ExpansionBudget::default();
        let (names, _) = names("T", &budget);
        let before = budget.snapshot();
        let remaining = budget.limits().counters[Counter::Work as usize]
            - before.counters[Counter::Work as usize];
        budget.charge(Counter::Work, remaining - 3).unwrap();
        let ty = Type::Tuple(vec![Type::Bool; 64]);
        let error = ty.semantic_eq(&Type::Bool, names.as_ref()).unwrap_err();
        let failed = budget.snapshot();
        let cause = failed.failure.clone().unwrap();
        assert_eq!(cause.kind, BudgetFailureKind::Limit);
        assert_eq!(cause.resource, BudgetResource::Counter(Counter::Work));
        assert_eq!(cause.requested, 64);
        assert_eq!(error, transport(TokenError::Budget(cause.clone())));
        assert_eq!(failed.temporary_bytes, 0);
        assert_eq!(failed.retained_bytes, before.retained_bytes);
        assert!(failed.counters[Counter::Work as usize] > before.counters[Counter::Work as usize]);
        assert_eq!(budget.check().unwrap_err(), cause);
    }

    #[test]
    fn name_free_comparisons_preserve_latched_allocation_failure() {
        let budget = ExpansionBudget::default();
        let (names, _) = names("T", &budget);
        let failure = budget.poison_allocation();
        let expected = transport(TokenError::Budget(failure.clone()));
        assert_eq!(
            Type::Bool
                .semantic_eq(&Type::Bool, names.as_ref())
                .unwrap_err(),
            expected
        );
        assert_eq!(
            FieldSelector::Deref
                .semantic_eq(&FieldSelector::Deref, names.as_ref())
                .unwrap_err(),
            expected
        );
        assert_eq!(
            region_eq(names.as_ref(), RegionName::Static, RegionName::Static).unwrap_err(),
            expected
        );
        assert_eq!(budget.snapshot().temporary_bytes, 0);
        assert_eq!(budget.check().unwrap_err(), failure);
    }
}
