//! Public ownership/key controls for the root-context name prerequisite.
//! These do not establish mixed-site resolution or source macro acceptance.
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use justrust_engine::compiler::semantic_name::{
    BuiltinAtom, LookupClass, NameError, NameStoreBuilder, Namespace, UnsupportedName,
};
use justrust_engine::compiler::semantic_path::{AstPath, PathAnchor};
use justrust_engine::compiler::source::PreparedLexInput;
use justrust_engine::compiler::token_primitives::{
    BudgetFailureKind, Counter, Edition, ExpansionBudget, InputToken, Token, TokenError,
    TokenSession, lex_prepared,
};

fn scanned(
    text: &str,
    edition: Edition,
    budget: &ExpansionBudget,
) -> (NameStoreBuilder, Vec<Token>) {
    let input = PreparedLexInput::identity(Path::new("name-control.rs"), text, budget).unwrap();
    let mut session = TokenSession::new(input.snapshot().clone(), budget.clone()).unwrap();
    let lexed = lex_prepared(&input, edition, &mut session).unwrap();
    assert_eq!(lexed.edition(), edition);
    let tokens = lexed
        .events()
        .iter()
        .filter_map(|event| match event.token {
            InputToken::Leaf(token) => Some(token),
            _ => None,
        })
        .collect();
    (NameStoreBuilder::new(session, edition).unwrap(), tokens)
}

#[test]
fn occurrence_origins_do_not_replace_semantic_keys_or_namespace() {
    let budget = ExpansionBudget::default();
    let (mut builder, tokens) = scanned("item item", Edition::E2024, &budget);
    let first = builder.from_token(tokens[0], Edition::E2024).unwrap();
    let second = builder.from_token(tokens[1], Edition::E2024).unwrap();
    assert_ne!(builder.span(first).unwrap(), builder.span(second).unwrap());
    let local = builder.key(first, LookupClass::Local).unwrap();
    let same_local = builder.key(second, LookupClass::Local).unwrap();
    assert_eq!(local, same_local);
    let value = builder
        .key(first, LookupClass::Item(Namespace::Value))
        .unwrap();
    let ty = builder
        .key(first, LookupClass::Item(Namespace::Type))
        .unwrap();
    let label = builder.key(first, LookupClass::Label).unwrap();
    let mut bindings = HashMap::new();
    bindings.insert(local, 1);
    bindings.insert(value, 2);
    bindings.insert(ty, 3);
    bindings.insert(label, 4);
    assert_eq!(bindings.len(), 4);
    assert_eq!(bindings.get(&same_local), Some(&1));
    let store = builder.freeze().unwrap();
    assert_eq!(store.key(second, LookupClass::Local).unwrap(), local);
    assert_eq!(store.spelling(first).unwrap(), "item");
    assert_eq!(store.data(first).unwrap().edition, Edition::E2024);
}

#[test]
fn closed_catalog_keys_bind_without_fabricating_ast_occurrences() {
    let (mut builder, tokens) = scanned("Copy Copy", Edition::E2024, &ExpansionBudget::default());
    let catalog = builder
        .catalog_key(BuiltinAtom::Copy, Namespace::Type)
        .unwrap();
    let source = builder.from_token(tokens[0], Edition::E2024).unwrap();
    let repeated = builder.from_token(tokens[1], Edition::E2024).unwrap();
    assert_eq!(
        catalog,
        builder
            .key(source, LookupClass::Item(Namespace::Type))
            .unwrap()
    );
    assert_eq!(
        catalog,
        builder
            .key(repeated, LookupClass::Item(Namespace::Type))
            .unwrap()
    );
    assert_ne!(
        catalog,
        builder
            .catalog_key(BuiltinAtom::Copy, Namespace::Value)
            .unwrap()
    );
    assert_ne!(catalog, builder.key(source, LookupClass::Local).unwrap());
    let (mut foreign, _) = scanned("Copy", Edition::E2024, &ExpansionBudget::default());
    assert_ne!(
        catalog,
        foreign
            .catalog_key(BuiltinAtom::Copy, Namespace::Type)
            .unwrap()
    );
    let names = builder.freeze().unwrap();
    assert_eq!(names.spelling(source).unwrap(), "Copy");
    assert_ne!(names.span(source).unwrap(), names.span(repeated).unwrap());
    assert_eq!(
        names
            .key(source, LookupClass::Item(Namespace::Type))
            .unwrap(),
        catalog
    );
}

#[test]
fn equal_spelling_and_allocation_order_do_not_merge_foreign_owners() {
    let (mut first, first_tokens) = scanned("item", Edition::E2024, &ExpansionBudget::default());
    let (mut second, second_tokens) = scanned("item", Edition::E2024, &ExpansionBudget::default());
    let a = first.from_token(first_tokens[0], Edition::E2024).unwrap();
    let b = second.from_token(second_tokens[0], Edition::E2024).unwrap();
    assert_ne!(
        first.key(a, LookupClass::Local).unwrap(),
        second.key(b, LookupClass::Local).unwrap()
    );
    assert!(matches!(
        first.check(b),
        Err(NameError::Token(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        second.check(a),
        Err(NameError::Token(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        first.from_token(second_tokens[0], Edition::E2024),
        Err(NameError::Token(TokenError::ForeignHandle))
    ));
    let first = first.freeze().unwrap();
    assert!(matches!(
        first.key(b, LookupClass::Local),
        Err(NameError::Token(TokenError::ForeignHandle))
    ));
}

#[test]
fn lexical_validity_does_not_silently_admit_new_name_classes() {
    for (text, reason) in [
        ("r#type", UnsupportedName::RawIdentifier),
        ("β", UnsupportedName::UnicodeIdentifier),
    ] {
        let (mut names, tokens) = scanned(text, Edition::E2024, &ExpansionBudget::default());
        assert!(
            matches!(names.from_token(tokens[0], Edition::E2024), Err(NameError::Unsupported(actual)) if actual == reason)
        );
    }
    let (mut names, tokens) = scanned("\"item\"", Edition::E2024, &ExpansionBudget::default());
    assert!(matches!(
        names.from_token(tokens[0], Edition::E2024),
        Err(NameError::ExpectedIdentifier)
    ));
}

#[test]
fn frozen_clone_owns_names_and_storage_until_last_program_owner_drops() {
    let budget = ExpansionBudget::default();
    let (mut builder, tokens) = scanned("item", Edition::E2021, &budget);
    let ident = builder.from_token(tokens[0], Edition::E2021).unwrap();
    let store = builder.freeze().unwrap();
    let retained = budget.snapshot().retained_bytes;
    assert!(retained > 0);
    let another_program_owner = Arc::clone(&store);
    drop(store);
    assert_eq!(budget.snapshot().retained_bytes, retained);
    assert_eq!(another_program_owner.spelling(ident).unwrap(), "item");
    assert_eq!(another_program_owner.edition(), Edition::E2021);
    drop(another_program_owner);
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
    assert!(budget.snapshot().counters[Counter::Work as usize] > 0);
}

#[test]
fn poisoned_owner_preserves_exact_typed_failure_and_releases_storage() {
    let budget = ExpansionBudget::default();
    let (mut builder, tokens) = scanned("item", Edition::E2024, &budget);
    let ident = builder.from_token(tokens[0], Edition::E2024).unwrap();
    let failure = budget.poison_allocation();
    assert_eq!(failure.kind, BudgetFailureKind::Allocation);
    assert!(
        matches!(builder.key(ident, LookupClass::Local), Err(NameError::Token(TokenError::Budget(actual))) if actual == failure)
    );
    assert!(
        matches!(builder.freeze(), Err(NameError::Token(TokenError::Budget(actual))) if actual == failure)
    );
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
    assert_eq!(budget.check().unwrap_err(), failure);
}

#[test]
fn structured_paths_compare_checked_names_not_printed_occurrences() {
    let budget = ExpansionBudget::default();
    let (mut names, tokens) = scanned("module item item", Edition::E2024, &budget);
    let module = names.from_token(tokens[0], Edition::E2024).unwrap();
    let first = names.from_token(tokens[1], Edition::E2024).unwrap();
    let second = names.from_token(tokens[2], Edition::E2024).unwrap();
    let prefix = AstPath::new(&names, PathAnchor::Crate, &[module]).unwrap();
    let a = prefix.push(&names, first).unwrap();
    let b = prefix.push(&names, second).unwrap();
    assert_eq!(a.key(Namespace::Value), b.key(Namespace::Value));
    assert_ne!(a.key(Namespace::Value), b.key(Namespace::Type));
    assert_ne!(
        a.key(Namespace::Value),
        a.with_anchor(PathAnchor::Relative).key(Namespace::Value)
    );
    assert_eq!(
        a.parent(&names).unwrap().key(Namespace::Type),
        prefix.key(Namespace::Type)
    );
    let suffix = AstPath::new(&names, PathAnchor::Relative, &[second]).unwrap();
    assert_eq!(
        prefix.join(&names, &suffix).unwrap().key(Namespace::Value),
        a.key(Namespace::Value)
    );
    assert!(matches!(
        prefix.join(&names, &prefix),
        Err(NameError::Token(TokenError::InvalidInput(_)))
    ));
    let names = names.freeze().unwrap();
    assert_eq!(
        a.display(names.as_ref()).unwrap().as_str(),
        "crate::module::item"
    );
}

#[test]
fn empty_root_paths_retain_owner_even_without_a_first_identifier() {
    let (first, _) = scanned("", Edition::E2024, &ExpansionBudget::default());
    let (second, _) = scanned("", Edition::E2024, &ExpansionBudget::default());
    let a = AstPath::new(&first, PathAnchor::Crate, &[]).unwrap();
    let b = AstPath::new(&second, PathAnchor::Crate, &[]).unwrap();
    assert_ne!(a.key(Namespace::Type), b.key(Namespace::Type));
    assert!(matches!(
        a.check(&second),
        Err(NameError::Token(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        a.join(&first, &b),
        Err(NameError::Token(TokenError::ForeignHandle))
    ));
}

#[test]
fn display_and_path_keys_retain_their_actual_backing_until_drop() {
    let budget = ExpansionBudget::default();
    let (mut names, tokens) = scanned("module item", Edition::E2024, &budget);
    let module = names.from_token(tokens[0], Edition::E2024).unwrap();
    let item = names.from_token(tokens[1], Edition::E2024).unwrap();
    let path = AstPath::new(&names, PathAnchor::Crate, &[module, item]).unwrap();
    let baseline = budget.snapshot().retained_bytes;
    let display = path.display(&names).unwrap();
    assert_eq!(display.as_str(), "crate::module::item");
    let displayed = budget.snapshot().retained_bytes;
    assert!(displayed >= baseline + display.as_str().len() as u64);
    let key = path.key(Namespace::Value);
    drop(path);
    assert_eq!(budget.snapshot().retained_bytes, displayed);
    drop(display);
    assert_eq!(budget.snapshot().retained_bytes, baseline);
    drop(key);
    assert!(budget.snapshot().retained_bytes < baseline);
    drop(names);
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
}
