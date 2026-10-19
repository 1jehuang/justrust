//! Public checked lexical-scope controls, not a mixed-site macro conformance claim.
//! Expansion admission proofs remain crate-private. Public marked-token rejection
//! is tested explicitly rather than manufacturing an unchecked scope anchor.
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use justrust_engine::compiler::semantic_name::{
    AstIdent, LookupClass, NameError, NameStore, NameStoreBuilder, Namespace, UnsupportedName,
};
use justrust_engine::compiler::semantic_scope::{ScopeArenaBuilder, ScopeError};
use justrust_engine::compiler::source::PreparedLexInput;
use justrust_engine::compiler::token_primitives::{
    BudgetFailure, BudgetFailureKind, BudgetResource, Counter, Edition, ExpansionBudget,
    ExpansionRecord, Ident, InputToken, Storage, Token, TokenError, TokenKind, TokenSession,
    Transparency, lex_prepared,
};

fn scanned(text: &str, budget: &ExpansionBudget) -> (NameStoreBuilder, Vec<Token>) {
    let input = PreparedLexInput::identity(Path::new("scope-control.rs"), text, budget).unwrap();
    let (mut session, cap) =
        TokenSession::standalone(input.snapshot().clone(), budget.clone()).unwrap();
    let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    let tokens = lexed
        .events()
        .iter()
        .filter_map(|event| match event.token {
            InputToken::Leaf(token) if matches!(token.kind, TokenKind::Ident(_)) => Some(token),
            _ => None,
        })
        .collect();
    (
        NameStoreBuilder::new(session, cap, Edition::E2024).unwrap(),
        tokens,
    )
}

fn names(text: &str, budget: &ExpansionBudget) -> (Arc<NameStore>, Vec<AstIdent>) {
    let (mut names, tokens) = scanned(text, budget);
    let occurrences = tokens
        .into_iter()
        .map(|token| names.from_token(token, Edition::E2024).unwrap())
        .collect();
    (names.freeze().unwrap(), occurrences)
}

/// The frozen store's owner receipt, checked to pay exactly the step capacity
/// actually allocated (not an assumed logical size).
fn exact_owner_receipt(store: &NameStore) -> u64 {
    let path = store.owner_path().expect("frozen store has an owner");
    assert_eq!(
        path.receipt_bytes(),
        justrust_engine::compiler::owner_path::owner_receipt_bytes(path.step_capacity())
    );
    path.receipt_bytes()
}

fn rejected<T>(result: Result<T, ScopeError>) -> ScopeError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected checked scope rejection"),
    }
}

fn budget_failure(error: ScopeError) -> BudgetFailure {
    match error {
        ScopeError::Token(TokenError::Budget(failure))
        | ScopeError::Name(NameError::Token(TokenError::Budget(failure))) => failure,
        other => panic!("expected original typed budget failure, got {other:?}"),
    }
}

#[test]
fn visibility_requires_the_exact_captured_frontier_and_function() {
    let (names, ids) = names("x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let (first, _) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    let child = scopes.enter(names.as_ref(), first).unwrap();
    let sibling = scopes.enter(names.as_ref(), first).unwrap();
    let (later, _) = scopes.bind(names.as_ref(), first, ids[1]).unwrap();
    let (inside, binding) = scopes.bind(names.as_ref(), child, ids[2]).unwrap();
    let other_function = scopes.function_root(names.as_ref()).unwrap();
    assert!(scopes.visible_from(first, inside).unwrap());
    assert!(scopes.visible_from(root, inside).unwrap());
    assert!(!scopes.visible_from(later, inside).unwrap());
    assert!(!scopes.visible_from(inside, later).unwrap());
    assert!(!scopes.visible_from(inside, sibling).unwrap());
    assert!(matches!(
        rejected(scopes.visible_from(first, other_function)),
        ScopeError::FunctionMismatch
    ));
    assert!(matches!(
        rejected(scopes.check_function(scopes.binding(binding).unwrap().scope)),
        ScopeError::InvalidHandle
    ));
    let mut foreign = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let foreign_root = foreign.function_root(names.as_ref()).unwrap();
    assert!(matches!(
        rejected(scopes.visible_from(first, foreign_root)),
        ScopeError::ForeignOwner
    ));
    let scopes = scopes.freeze().unwrap();
    assert!(scopes.visible_from(first, inside).unwrap());
    assert!(!scopes.visible_from(later, inside).unwrap());
    assert_eq!(
        scopes.binding_function(binding).unwrap(),
        scopes.function(root).unwrap()
    );
}

#[test]
fn recorded_reference_projection_retains_exact_frontier_function_and_class() {
    let (names, ids) = names("x x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let function = scopes.function(root).unwrap();
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), root, ids[1])
            .unwrap(),
        None
    );
    let (at, binding) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    assert_eq!(
        scopes.bind(names.as_ref(), root, ids[0]).unwrap(),
        (at, binding)
    );
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), at, ids[2])
            .unwrap(),
        Some(binding)
    );
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), at, ids[2])
            .unwrap(),
        Some(binding)
    );
    let scopes = scopes.freeze().unwrap();
    assert_eq!(
        scopes.reference_frontier(names.as_ref(), ids[1]).unwrap(),
        root
    );
    assert_eq!(
        scopes.reference_frontier(names.as_ref(), ids[2]).unwrap(),
        at
    );
    assert_eq!(
        scopes.reference_function(names.as_ref(), ids[1]).unwrap(),
        function
    );
    assert_eq!(
        scopes.reference_function(names.as_ref(), ids[2]).unwrap(),
        function
    );
    let classes = [
        LookupClass::Local,
        LookupClass::Item(Namespace::Value),
        LookupClass::Item(Namespace::Type),
        LookupClass::Item(Namespace::Macro),
        LookupClass::Member,
        LookupClass::Label,
        LookupClass::TypeParameter,
        LookupClass::LifetimeParameter,
    ];
    let mut keys = Vec::new();
    for class in classes {
        let projected = scopes
            .project_reference(names.as_ref(), ids[2], class)
            .unwrap();
        assert_eq!(projected.frontier, at);
        assert_eq!(projected.key, names.key(ids[2], class).unwrap());
        assert!(!keys.contains(&projected.key));
        keys.push(projected.key);
    }
    assert!(matches!(
        rejected(scopes.project_reference(names.as_ref(), ids[3], LookupClass::Local)),
        ScopeError::UnrecordedOccurrence
    ));
    assert!(matches!(
        rejected(scopes.reference_function(names.as_ref(), ids[3])),
        ScopeError::UnrecordedOccurrence
    ));
    assert!(matches!(
        rejected(scopes.reference_frontier(names.as_ref(), ids[3])),
        ScopeError::UnrecordedOccurrence
    ));
}

#[test]
fn source_order_shadowing_keeps_prior_frontiers_and_unresolved_references_exact() {
    let (names, ids) = names("x x x x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), root, ids[1])
            .unwrap(),
        None
    );
    let (first_point, first) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), first_point, ids[2])
            .unwrap(),
        Some(first)
    );
    let (second_point, second) = scopes.bind(names.as_ref(), first_point, ids[3]).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        names.key(ids[0], LookupClass::Local).unwrap(),
        names.key(ids[3], LookupClass::Local).unwrap()
    );
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), first_point, ids[4])
            .unwrap(),
        Some(first)
    );
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), second_point, ids[5])
            .unwrap(),
        Some(second)
    );
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[1]).unwrap(),
        None,
        "later bind cannot repair an earlier unresolved occurrence"
    );
    let scopes = scopes.freeze().unwrap();
    assert_eq!(
        scopes.declaration_for(names.as_ref(), ids[0]).unwrap(),
        first
    );
    assert_eq!(
        scopes.declaration_for(names.as_ref(), ids[3]).unwrap(),
        second
    );
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[2]).unwrap(),
        Some(first)
    );
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[4]).unwrap(),
        Some(first)
    );
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[5]).unwrap(),
        Some(second)
    );
    assert_eq!(scopes.reference_for(names.as_ref(), ids[1]).unwrap(), None);
}

#[test]
fn child_scope_captures_exact_entry_frontier_not_later_parent_shadowing() {
    let (names, ids) = names("x x x x x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let function = scopes.function(root).unwrap();
    let (parent, original) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    let child = scopes.enter(names.as_ref(), parent).unwrap();
    let (later_parent, shadow) = scopes.bind(names.as_ref(), parent, ids[1]).unwrap();
    assert_eq!(scopes.function(child).unwrap(), function);
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), child, ids[2])
            .unwrap(),
        Some(original)
    );
    let (child_shadow_point, child_shadow) = scopes.bind(names.as_ref(), child, ids[3]).unwrap();
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), child_shadow_point, ids[4])
            .unwrap(),
        Some(child_shadow)
    );
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), later_parent, ids[5])
            .unwrap(),
        Some(shadow)
    );
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), parent, ids[6])
            .unwrap(),
        Some(original)
    );
    assert_ne!(
        scopes.binding(original).unwrap().scope,
        scopes.binding(child_shadow).unwrap().scope
    );
    let scopes = scopes.freeze().unwrap();
    scopes.check_function(function).unwrap();
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[2]).unwrap(),
        Some(original)
    );
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[4]).unwrap(),
        Some(child_shadow)
    );
}

#[test]
fn separate_functions_never_inherit_each_others_same_atom_binders() {
    let (names, ids) = names("x x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let first_root = scopes.function_root(names.as_ref()).unwrap();
    let second_root = scopes.function_root(names.as_ref()).unwrap();
    let first_function = scopes.function(first_root).unwrap();
    let second_function = scopes.function(second_root).unwrap();
    assert_ne!(first_function, second_function);
    let (first_point, first) = scopes.bind(names.as_ref(), first_root, ids[0]).unwrap();
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), second_root, ids[1])
            .unwrap(),
        None
    );
    let (second_point, second) = scopes.bind(names.as_ref(), second_root, ids[2]).unwrap();
    assert_ne!(first, second);
    assert_eq!(scopes.binding(first).unwrap().scope, first_function);
    assert_eq!(scopes.binding(second).unwrap().scope, second_function);
    assert_eq!(scopes.function(first_point).unwrap(), first_function);
    assert_eq!(scopes.function(second_point).unwrap(), second_function);
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), second_point, ids[3])
            .unwrap(),
        Some(second)
    );
    let scopes = scopes.freeze().unwrap();
    scopes.check_function(first_function).unwrap();
    scopes.check_function(second_function).unwrap();
    assert_eq!(scopes.reference_for(names.as_ref(), ids[1]).unwrap(), None);
}

#[test]
fn equal_span_and_atom_do_not_replace_checked_occurrence_or_binding_identity() {
    let (mut names, tokens) = scanned("x", &ExpansionBudget::default());
    let first = names.from_token(tokens[0], Edition::E2024).unwrap();
    let second = names.from_token(tokens[0], Edition::E2024).unwrap();
    assert_eq!(names.span(first).unwrap(), names.span(second).unwrap());
    assert_eq!(
        names.key(first, LookupClass::Local).unwrap(),
        names.key(second, LookupClass::Local).unwrap()
    );
    let names = names.freeze().unwrap();
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let (first_point, first_binding) = scopes.bind(names.as_ref(), root, first).unwrap();
    assert!(matches!(
        scopes.declaration_for(names.as_ref(), second),
        Err(ScopeError::UnrecordedOccurrence)
    ));
    let (_, second_binding) = scopes.bind(names.as_ref(), first_point, second).unwrap();
    assert_ne!(first_binding, second_binding);
    assert_eq!(
        scopes.declaration_for(names.as_ref(), first).unwrap(),
        first_binding
    );
    assert_eq!(
        scopes.declaration_for(names.as_ref(), second).unwrap(),
        second_binding
    );
}

#[test]
fn unrecorded_occurrence_is_not_an_explicitly_recorded_unresolved_reference() {
    let (names, ids) = names("x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    assert!(matches!(
        scopes.reference_for(names.as_ref(), ids[0]),
        Err(ScopeError::UnrecordedOccurrence)
    ));
    assert!(matches!(
        scopes.declaration_for(names.as_ref(), ids[1]),
        Err(ScopeError::UnrecordedOccurrence)
    ));
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), root, ids[0])
            .unwrap(),
        None
    );
    let scopes = scopes.freeze().unwrap();
    assert_eq!(scopes.reference_for(names.as_ref(), ids[0]).unwrap(), None);
    assert!(matches!(
        scopes.reference_for(names.as_ref(), ids[2]),
        Err(ScopeError::UnrecordedOccurrence)
    ));
    assert!(matches!(
        scopes.declaration_for(names.as_ref(), ids[0]),
        Err(ScopeError::UnrecordedOccurrence)
    ));
}

#[test]
fn conflicting_reference_reassociation_and_rebinding_do_not_overwrite_prior_facts() {
    let (names, ids) = names("x x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let (first_point, first) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    assert_eq!(
        scopes
            .resolve_reference(names.as_ref(), first_point, ids[1])
            .unwrap(),
        Some(first)
    );
    let (second_point, _) = scopes.bind(names.as_ref(), first_point, ids[2]).unwrap();
    assert!(matches!(
        scopes.resolve_reference(names.as_ref(), second_point, ids[1]),
        Err(ScopeError::ConflictingAssociation)
    ));
    assert!(matches!(
        scopes.bind(names.as_ref(), second_point, ids[0]),
        Err(ScopeError::ConflictingAssociation)
    ));
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[1]).unwrap(),
        Some(first)
    );
    assert_eq!(
        scopes.declaration_for(names.as_ref(), ids[0]).unwrap(),
        first
    );
    let scopes = scopes.freeze().unwrap();
    assert_eq!(
        scopes.reference_for(names.as_ref(), ids[1]).unwrap(),
        Some(first)
    );
}

#[test]
fn arena_owner_is_checked_even_when_name_session_and_allocation_order_match() {
    let (names, ids) = names("x x", &ExpansionBudget::default());
    let mut first = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let mut second = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let first_root = first.function_root(names.as_ref()).unwrap();
    let second_root = second.function_root(names.as_ref()).unwrap();
    let first_function = first.function(first_root).unwrap();
    let second_function = second.function(second_root).unwrap();
    let (_, foreign_binding) = second.bind(names.as_ref(), second_root, ids[0]).unwrap();
    assert_ne!(first_root, second_root);
    assert_ne!(first_function, second_function);
    assert!(matches!(
        first.function(second_root),
        Err(ScopeError::ForeignOwner)
    ));
    assert!(matches!(
        first.check_function(second_function),
        Err(ScopeError::ForeignOwner)
    ));
    assert!(matches!(
        first.enter(names.as_ref(), second_root),
        Err(ScopeError::ForeignOwner)
    ));
    assert!(matches!(
        first.bind(names.as_ref(), second_root, ids[0]),
        Err(ScopeError::ForeignOwner)
    ));
    assert!(matches!(
        first.resolve_reference(names.as_ref(), second_root, ids[1]),
        Err(ScopeError::ForeignOwner)
    ));
    assert!(matches!(
        first.binding(foreign_binding),
        Err(ScopeError::ForeignOwner)
    ));
    let first = first.freeze().unwrap();
    assert!(matches!(
        first.check_function(second_function),
        Err(ScopeError::ForeignOwner)
    ));
    assert!(matches!(
        first.binding(foreign_binding),
        Err(ScopeError::ForeignOwner)
    ));
    first.check_function(first_function).unwrap();
}

#[test]
fn name_session_is_checked_independently_from_shared_budget_and_equal_spelling() {
    let budget = ExpansionBudget::default();
    let (own_names, own) = names("x x", &budget);
    let (foreign_names, foreign) = names("x x", &budget);
    let mut scopes = ScopeArenaBuilder::for_names(own_names.as_ref()).unwrap();
    let root = scopes.function_root(own_names.as_ref()).unwrap();
    let (_, binding) = scopes.bind(own_names.as_ref(), root, own[0]).unwrap();
    assert!(matches!(
        scopes.check_names(foreign_names.as_ref()),
        Err(ScopeError::ForeignNames)
    ));
    assert!(matches!(
        scopes.function_root(foreign_names.as_ref()),
        Err(ScopeError::ForeignNames)
    ));
    assert!(matches!(
        scopes.enter(foreign_names.as_ref(), root),
        Err(ScopeError::ForeignNames)
    ));
    assert!(matches!(
        scopes.bind(foreign_names.as_ref(), root, foreign[0]),
        Err(ScopeError::ForeignNames)
    ));
    assert!(matches!(
        scopes.resolve_reference(foreign_names.as_ref(), root, foreign[1]),
        Err(ScopeError::ForeignNames)
    ));
    assert!(matches!(
        scopes.declaration_for(foreign_names.as_ref(), foreign[0]),
        Err(ScopeError::ForeignNames)
    ));
    let scopes = scopes.freeze().unwrap();
    assert!(matches!(
        scopes.check_names(foreign_names.as_ref()),
        Err(ScopeError::ForeignNames)
    ));
    assert!(matches!(
        scopes.reference_for(foreign_names.as_ref(), foreign[1]),
        Err(ScopeError::ForeignNames)
    ));
    assert_eq!(
        scopes.declaration_for(own_names.as_ref(), own[0]).unwrap(),
        binding
    );
}

#[test]
fn foreign_occurrence_cannot_be_smuggled_through_the_correct_name_store_argument() {
    let (own_names, own) = names("x", &ExpansionBudget::default());
    let (foreign_names, foreign) = names("x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(own_names.as_ref()).unwrap();
    let root = scopes.function_root(own_names.as_ref()).unwrap();
    assert!(matches!(
        scopes.bind(own_names.as_ref(), root, foreign[0]),
        Err(ScopeError::Name(NameError::Token(
            TokenError::ForeignHandle
        )))
    ));
    assert!(matches!(
        scopes.resolve_reference(own_names.as_ref(), root, foreign[0]),
        Err(ScopeError::Name(NameError::Token(
            TokenError::ForeignHandle
        )))
    ));
    let (_, binding) = scopes.bind(own_names.as_ref(), root, own[0]).unwrap();
    let scopes = scopes.freeze().unwrap();
    assert!(matches!(
        scopes.declaration_for(own_names.as_ref(), foreign[0]),
        Err(ScopeError::Name(NameError::Token(
            TokenError::ForeignHandle
        )))
    ));
    assert!(matches!(
        scopes.reference_for(own_names.as_ref(), foreign[0]),
        Err(ScopeError::Name(NameError::Token(
            TokenError::ForeignHandle
        )))
    ));
    assert_eq!(
        scopes.declaration_for(own_names.as_ref(), own[0]).unwrap(),
        binding
    );
    assert_eq!(foreign_names.spelling(foreign[0]).unwrap(), "x");
}

#[test]
fn empty_arena_still_retains_exact_name_owner_association() {
    let (own, _) = names("", &ExpansionBudget::default());
    let (foreign, _) = names("", &ExpansionBudget::default());
    let scopes = ScopeArenaBuilder::for_names(own.as_ref()).unwrap();
    scopes.check_names(own.as_ref()).unwrap();
    assert!(matches!(
        scopes.check_names(foreign.as_ref()),
        Err(ScopeError::ForeignNames)
    ));
    let scopes = scopes.freeze().unwrap();
    scopes.check_names(own.as_ref()).unwrap();
    assert!(matches!(
        scopes.check_names(foreign.as_ref()),
        Err(ScopeError::ForeignNames)
    ));
}

#[test]
fn recorded_binding_metadata_retains_the_actual_declaration_occurrence() {
    let (names, ids) = names("x x", &ExpansionBudget::default());
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let (_, binding) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    let scopes = scopes.freeze().unwrap();
    let data = scopes.binding(binding).unwrap();
    assert_eq!(
        names.span(data.declaration).unwrap(),
        names.span(ids[0]).unwrap()
    );
    assert_ne!(
        names.span(data.declaration).unwrap(),
        names.span(ids[1]).unwrap()
    );
    assert_eq!(names.spelling(data.declaration).unwrap(), "x");
}

#[test]
fn transition_limit_rejects_before_allocating_and_latches_exact_failure() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("x", &budget);
    let receipt = exact_owner_receipt(&names);
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let used = budget.snapshot().counters[Counter::ScopeTransitions as usize];
    budget
        .charge(
            Counter::ScopeTransitions,
            budget.limits().counters[Counter::ScopeTransitions as usize] - used,
        )
        .unwrap();
    let before = budget.snapshot();
    let failure = budget_failure(rejected(scopes.enter(names.as_ref(), root)));
    assert_eq!(failure.kind, BudgetFailureKind::Limit);
    assert_eq!(
        failure.resource,
        BudgetResource::Counter(Counter::ScopeTransitions)
    );
    assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
    assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    assert_eq!(
        budget_failure(rejected(scopes.bind(names.as_ref(), root, ids[0]))),
        failure
    );
    assert_eq!(budget_failure(rejected(scopes.freeze())), failure);
    drop(names);
    // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md):
    // it is paid once at freeze and never refunded.
    assert_eq!(budget.snapshot().retained_bytes, receipt);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
}

#[test]
fn constructor_byte_admission_rejects_before_allocating_scope_payload() {
    let budget = ExpansionBudget::default();
    let (names, _) = names("x", &budget);
    let receipt = exact_owner_receipt(&names);
    let live = budget.snapshot();
    let ballast = budget
        .reserve(
            Storage::Retained,
            budget.limits().bytes - live.retained_bytes - live.temporary_bytes,
        )
        .unwrap();
    let before = budget.snapshot();
    let failure = budget_failure(rejected(ScopeArenaBuilder::for_names(names.as_ref())));
    assert_eq!(failure.kind, BudgetFailureKind::Limit);
    assert_eq!(failure.resource, BudgetResource::Bytes);
    assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
    assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    assert_eq!(
        budget_failure(rejected(ScopeArenaBuilder::for_names(names.as_ref()))),
        failure
    );
    drop(ballast);
    drop(names);
    // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md):
    // it is paid once at freeze and never refunded.
    assert_eq!(budget.snapshot().retained_bytes, receipt);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
}

#[test]
fn poisoned_frozen_arena_preserves_failure_but_id_equality_and_hash_are_pure() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("x x", &budget);
    let receipt = exact_owner_receipt(&names);
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let root = scopes.function_root(names.as_ref()).unwrap();
    let function = scopes.function(root).unwrap();
    let (point, binding) = scopes.bind(names.as_ref(), root, ids[0]).unwrap();
    scopes
        .resolve_reference(names.as_ref(), point, ids[1])
        .unwrap();
    let scopes = scopes.freeze().unwrap();
    let mut keys = HashMap::new();
    keys.insert(binding, 7);
    let failure = budget.poison_allocation();
    let before = budget.snapshot();
    assert_eq!(keys.get(&binding), Some(&7));
    assert_eq!(
        budget.snapshot(),
        before,
        "pure ID equality/hash cannot consult the poisoned ledger"
    );
    assert_eq!(budget_failure(rejected(scopes.binding(binding))), failure);
    assert_eq!(
        budget_failure(rejected(scopes.check_function(function))),
        failure
    );
    assert_eq!(
        budget_failure(rejected(scopes.check_names(names.as_ref()))),
        failure
    );
    assert_eq!(
        budget_failure(rejected(scopes.declaration_for(names.as_ref(), ids[0]))),
        failure
    );
    assert_eq!(
        budget_failure(rejected(scopes.reference_for(names.as_ref(), ids[1]))),
        failure
    );
    drop(scopes);
    drop(names);
    // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md):
    // it is paid once at freeze and never refunded.
    assert_eq!(budget.snapshot().retained_bytes, receipt);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
}

#[test]
fn frozen_clone_keeps_flat_scope_storage_alive_until_last_owner_drops() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("x", &budget);
    let receipt = exact_owner_receipt(&names);
    let names_only = budget.snapshot().retained_bytes;
    let mut scopes = ScopeArenaBuilder::for_names(names.as_ref()).unwrap();
    let mut frontier = scopes.function_root(names.as_ref()).unwrap();
    for _ in 0..512 {
        frontier = scopes.enter(names.as_ref(), frontier).unwrap();
    }
    let (_, binding) = scopes.bind(names.as_ref(), frontier, ids[0]).unwrap();
    let scopes = scopes.freeze().unwrap();
    let retained = budget.snapshot().retained_bytes;
    assert!(retained > names_only);
    let last = Arc::clone(&scopes);
    drop(scopes);
    assert_eq!(budget.snapshot().retained_bytes, retained);
    assert_eq!(
        names
            .spelling(last.binding(binding).unwrap().declaration)
            .unwrap(),
        "x"
    );
    drop(last);
    assert_eq!(budget.snapshot().retained_bytes, names_only);
    drop(names);
    // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md):
    // it is paid once at freeze and never refunded.
    assert_eq!(budget.snapshot().retained_bytes, receipt);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
    assert!(budget.snapshot().counters[Counter::ScopeTransitions as usize] >= 512);
}

#[test]
fn public_name_gate_cannot_mint_unproved_marked_or_expanded_scope_occurrences() {
    let budget = ExpansionBudget::default();
    let input =
        PreparedLexInput::identity(Path::new("scope-mark-control.rs"), "x", &budget).unwrap();
    let (mut session, cap) =
        TokenSession::standalone(input.snapshot().clone(), budget.clone()).unwrap();
    let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    let original = lexed
        .events()
        .iter()
        .find_map(|event| match event.token {
            InputToken::Leaf(token) if matches!(token.kind, TokenKind::Ident(_)) => Some(token),
            _ => None,
        })
        .unwrap();
    let TokenKind::Ident(atom) = original.kind else {
        unreachable!()
    };
    let defining_crate = session
        .register_crate_root(session.sources().file(0).unwrap())
        .unwrap();
    let expansion = session
        .add_expansion(ExpansionRecord {
            definition: original.span,
            call: original.span,
            parent: None,
            defining_crate,
            edition: Edition::E2024,
        })
        .unwrap();
    let root = session.root_context();
    let mut marked_tokens = Vec::new();
    for transparency in [Transparency::SemiTransparent, Transparency::Opaque] {
        let context = session.apply_mark(root, expansion, transparency).unwrap();
        let span = session
            .with_context(original.span, context, Some(expansion))
            .unwrap();
        marked_tokens.push(Token {
            span,
            kind: TokenKind::Ident(Ident { context, ..atom }),
        });
    }
    let captured_span = session
        .with_context(original.span, root, Some(expansion))
        .unwrap();
    let unproved_capture = Token {
        span: captured_span,
        kind: original.kind,
    };
    let mut names = NameStoreBuilder::new(session, cap, Edition::E2024).unwrap();
    for token in marked_tokens {
        assert!(matches!(
            names.from_token(token, Edition::E2024),
            Err(NameError::Unsupported(UnsupportedName::NonRootContext))
        ));
    }
    assert!(matches!(
        names.from_token(unproved_capture, Edition::E2024),
        Err(NameError::Unsupported(UnsupportedName::ExpandedOccurrence))
    ));
    let original_name = names.from_token(original, Edition::E2024).unwrap();
    let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
    let root = scopes.function_root(&names).unwrap();
    let (_, binding) = scopes.bind(&names, root, original_name).unwrap();
    assert_eq!(
        names
            .spelling(scopes.binding(binding).unwrap().declaration)
            .unwrap(),
        "x"
    );
}
