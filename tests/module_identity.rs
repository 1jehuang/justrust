//! External checked module-owner and privacy controls, not Rust/module admission.
//! These structural fixtures use scanned names and explicit resource ledgers.
use justrust_engine::compiler::ir::Visibility;
use justrust_engine::compiler::module_identity::{CatalogModule, ModuleArenaBuilder, ModuleError};
use justrust_engine::compiler::semantic_name::{AstIdent, NameError, NameStoreBuilder};
use justrust_engine::compiler::source::PreparedLexInput;
use justrust_engine::compiler::token_primitives::{
    BudgetFailureKind, BudgetResource, Counter, Edition, ExpansionBudget, ExpansionLimits,
    InputToken, TokenError, TokenSession, lex_prepared,
};
use std::path::Path;
use std::sync::Arc;

fn names(text: &str, budget: &ExpansionBudget) -> (NameStoreBuilder, Vec<AstIdent>) {
    let input = PreparedLexInput::identity(Path::new("module-control.rs"), text, budget).unwrap();
    let (mut session, cap) =
        TokenSession::standalone(input.snapshot().clone(), budget.clone()).unwrap();
    let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    let tokens = lexed
        .events()
        .iter()
        .map(|event| match event.token {
            InputToken::Leaf(token) => token,
            _ => panic!("identifier-only fixture"),
        })
        .collect::<Vec<_>>();
    let mut names = NameStoreBuilder::new(session, cap, Edition::E2024).unwrap();
    let ids = tokens
        .into_iter()
        .map(|token| names.from_token(token, Edition::E2024).unwrap())
        .collect();
    (names, ids)
}

#[test]
fn source_and_catalog_roots_are_real_disjoint_checked_modules() {
    let budget = ExpansionBudget::default();
    let (names, _) = names("item", &budget);
    let builder = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(builder.budget().same_authority(&budget));
    let source = builder.source_root().unwrap();
    let core = builder.core_root();
    assert_ne!(source, core);
    assert_eq!(builder.parent(source).unwrap(), None);
    assert_eq!(builder.parent(core).unwrap(), None);
    assert_eq!(builder.root_of(source).unwrap(), source);
    assert_eq!(builder.root_of(core).unwrap(), core);
    assert!(builder.is_descendant(source, source).unwrap());
    assert!(!builder.is_descendant(source, core).unwrap());
    assert!(!builder.is_descendant(core, source).unwrap());
    assert_eq!(builder.catalog_module(CatalogModule::Root), core);
    for which in [
        CatalogModule::Marker,
        CatalogModule::Clone,
        CatalogModule::Ops,
    ] {
        let child = builder.catalog_module(which);
        assert_ne!(child, source);
        assert_ne!(child, core);
        assert_eq!(builder.parent(child).unwrap(), Some(core));
        assert_eq!(builder.root_of(child).unwrap(), core);
        assert!(builder.is_descendant(child, core).unwrap());
        assert!(!builder.is_descendant(child, source).unwrap());
    }
    let arena = builder.freeze().unwrap();
    assert_eq!(arena.source_root().unwrap(), source);
    assert_eq!(arena.core_root(), core);
    assert_eq!(arena.root_of(source).unwrap(), source);
    assert_eq!(arena.parent(core).unwrap(), None);
}

#[test]
fn catalog_only_owner_has_no_source_sentinel_or_implicit_budget() {
    let budget = ExpansionBudget::default();
    let builder = ModuleArenaBuilder::for_catalog(
        budget.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &budget.clone(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(builder.budget().same_authority(&budget));
    assert!(matches!(
        builder.source_root(),
        Err(ModuleError::SourceUnavailable)
    ));
    let core = builder.core_root();
    assert_eq!(builder.root_of(core).unwrap(), core);
    let arena = builder.freeze().unwrap();
    assert!(matches!(
        arena.source_root(),
        Err(ModuleError::SourceUnavailable)
    ));
    assert!(arena.budget().same_authority(&budget));
    assert_eq!(arena.catalog_module(CatalogModule::Root), core);
}

#[test]
fn child_identity_is_parent_plus_checked_name_not_occurrence_or_spelling_alone() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("left right leaf leaf", &budget);
    let mut modules = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let root = modules.source_root().unwrap();
    let left = modules.intern_child(&names, root, ids[0]).unwrap();
    let right = modules.intern_child(&names, root, ids[1]).unwrap();
    let first = modules.intern_child(&names, left, ids[2]).unwrap();
    let same = modules.intern_child(&names, left, ids[3]).unwrap();
    let other = modules.intern_child(&names, right, ids[2]).unwrap();
    assert_eq!(first, same);
    assert_ne!(first, other);
    assert_eq!(
        modules.lookup_child(&names, left, ids[3]).unwrap(),
        Some(first)
    );
    assert_eq!(
        modules.lookup_child(&names, right, ids[3]).unwrap(),
        Some(other)
    );
    assert_eq!(modules.lookup_child(&names, root, ids[3]).unwrap(), None);
    assert_eq!(modules.parent(first).unwrap(), Some(left));
    assert_eq!(modules.root_of(first).unwrap(), root);
    assert!(modules.is_descendant(first, left).unwrap());
    assert!(!modules.is_descendant(left, first).unwrap());
    assert!(!modules.is_descendant(other, left).unwrap());
    let names = names.freeze().unwrap();
    let arena = modules.freeze().unwrap();
    assert_eq!(
        arena.lookup_child(names.as_ref(), left, ids[3]).unwrap(),
        Some(first)
    );
}

#[test]
fn same_source_name_owner_does_not_merge_independent_module_arenas() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("child", &budget);
    let mut first = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let mut second = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let a = first.source_root().unwrap();
    let b = second.source_root().unwrap();
    assert_ne!(a, b);
    assert_ne!(first.core_root(), second.core_root());
    assert!(matches!(first.check(b), Err(ModuleError::ForeignOwner)));
    assert!(matches!(first.parent(b), Err(ModuleError::ForeignOwner)));
    assert!(matches!(first.root_of(b), Err(ModuleError::ForeignOwner)));
    assert!(matches!(
        first.is_descendant(a, b),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        first.is_descendant(b, a),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        first.intern_child(&names, b, ids[0]),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        second.intern_child(&names, a, ids[0]),
        Err(ModuleError::ForeignOwner)
    ));
    let arena = first.freeze().unwrap();
    assert!(matches!(arena.check(b), Err(ModuleError::ForeignOwner)));
    assert!(matches!(
        arena.lookup_child(&names, b, ids[0]),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(
        budget.check().is_ok(),
        "foreign caller input is not a resource poison"
    );
}

#[test]
fn foreign_name_store_cannot_supply_same_spelling_module_authority() {
    let budget = ExpansionBudget::default();
    let (own, ids) = names("child", &budget);
    let (foreign, other) = names("child", &budget);
    let mut modules = ModuleArenaBuilder::for_source(
        &own,
        justrust_engine::compiler::owner_path::standalone_issued(
            &own.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let root = modules.source_root().unwrap();
    let child = modules.intern_child(&own, root, ids[0]).unwrap();
    assert!(matches!(
        modules.intern_child(&foreign, root, other[0]),
        Err(ModuleError::ForeignNames)
    ));
    assert!(matches!(
        modules.lookup_child(&foreign, root, other[0]),
        Err(ModuleError::ForeignNames)
    ));
    assert!(
        matches!(
            modules.intern_child(&own, root, other[0]),
            Err(ModuleError::Name(NameError::Token(
                TokenError::ForeignHandle
            )))
        ),
        "foreign AstIdent cannot be laundered through own store"
    );
    assert_eq!(
        modules.lookup_child(&own, root, ids[0]).unwrap(),
        Some(child)
    );
    let arena = modules.freeze().unwrap();
    assert!(matches!(
        arena.lookup_child(&foreign, root, other[0]),
        Err(ModuleError::ForeignNames)
    ));
    assert!(budget.check().is_ok());
}

#[test]
fn source_children_cannot_attach_to_catalog_roots_or_descendants() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("child", &budget);
    let mut modules = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    for parent in [
        modules.core_root(),
        modules.catalog_module(CatalogModule::Marker),
        modules.catalog_module(CatalogModule::Clone),
        modules.catalog_module(CatalogModule::Ops),
    ] {
        assert!(matches!(
            modules.intern_child(&names, parent, ids[0]),
            Err(ModuleError::InvalidParent)
        ));
    }
    let source = modules.source_root().unwrap();
    assert_eq!(modules.lookup_child(&names, source, ids[0]).unwrap(), None);
    assert!(modules.intern_child(&names, source, ids[0]).is_ok());
    assert!(budget.check().is_ok());
}

#[test]
fn privacy_restrictions_require_same_root_ancestor_at_schema_admission() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("left right leaf", &budget);
    let mut modules = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let root = modules.source_root().unwrap();
    let left = modules.intern_child(&names, root, ids[0]).unwrap();
    let right = modules.intern_child(&names, root, ids[1]).unwrap();
    let leaf = modules.intern_child(&names, left, ids[2]).unwrap();
    let core = modules.core_root();
    let arena = modules.freeze().unwrap();
    for scope in [root, left, leaf] {
        Visibility::Restricted(scope)
            .validate_for(&arena, leaf)
            .unwrap();
    }
    for scope in [right, core] {
        assert!(matches!(
            Visibility::Restricted(scope).validate_for(&arena, leaf),
            Err(ModuleError::InvalidRestriction)
        ));
    }
    assert!(
        Visibility::Restricted(left)
            .accessible_from(&arena, leaf)
            .unwrap()
    );
    assert!(
        !Visibility::Restricted(left)
            .accessible_from(&arena, right)
            .unwrap()
    );
    assert!(
        !Visibility::Restricted(left)
            .accessible_from(&arena, core)
            .unwrap()
    );
    assert!(
        !Visibility::Restricted(core)
            .accessible_from(&arena, leaf)
            .unwrap()
    );
    assert!(Visibility::Public.accessible_from(&arena, leaf).unwrap());
}

#[test]
fn reexport_cannot_widen_restricted_audience_or_cross_roots() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("left right leaf", &budget);
    let mut modules = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let root = modules.source_root().unwrap();
    let left = modules.intern_child(&names, root, ids[0]).unwrap();
    let right = modules.intern_child(&names, root, ids[1]).unwrap();
    let leaf = modules.intern_child(&names, left, ids[2]).unwrap();
    let core = modules.core_root();
    let arena = modules.freeze().unwrap();
    let original = Visibility::Restricted(left);
    for scope in [left, leaf] {
        assert!(
            original
                .allows_reexport(&arena, &Visibility::Restricted(scope))
                .unwrap()
        );
    }
    for scope in [root, right, core] {
        assert!(
            !original
                .allows_reexport(&arena, &Visibility::Restricted(scope))
                .unwrap()
        );
    }
    assert!(
        !original
            .allows_reexport(&arena, &Visibility::Public)
            .unwrap()
    );
    assert!(
        Visibility::Public
            .allows_reexport(&arena, &original)
            .unwrap()
    );
}

#[test]
fn public_visibility_never_shortcuts_foreign_module_validation() {
    let budget = ExpansionBudget::default();
    let own = ModuleArenaBuilder::for_catalog(
        budget.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &budget.clone(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap()
    .freeze()
    .unwrap();
    let other = ModuleArenaBuilder::for_catalog(
        budget.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &budget.clone(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap()
    .freeze()
    .unwrap();
    let foreign = other.core_root();
    let local = own.core_root();
    assert!(matches!(
        Visibility::Public.validate_for(&own, foreign),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        Visibility::Public.accessible_from(&own, foreign),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        Visibility::Public.allows_reexport(&own, &Visibility::Restricted(foreign)),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        Visibility::Restricted(foreign).allows_reexport(&own, &Visibility::Public),
        Err(ModuleError::ForeignOwner)
    ));
    assert!(matches!(
        Visibility::Restricted(foreign).validate_for(&own, local),
        Err(ModuleError::ForeignOwner)
    ));
    Visibility::Public.validate_for(&own, local).unwrap();
}

#[test]
fn deep_flat_ancestry_has_no_new_semantic_depth_cap() {
    let budget = ExpansionBudget::default();
    let (names, ids) = names("child", &budget);
    let mut modules = ModuleArenaBuilder::for_source(
        &names,
        justrust_engine::compiler::owner_path::standalone_issued(
            &names.budget(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap();
    let root = modules.source_root().unwrap();
    let mut deepest = root;
    for _ in 0..600 {
        deepest = modules.intern_child(&names, deepest, ids[0]).unwrap();
    }
    assert_eq!(modules.root_of(deepest).unwrap(), root);
    assert!(modules.is_descendant(deepest, root).unwrap());
    assert!(!modules.is_descendant(root, deepest).unwrap());
    let arena = modules.freeze().unwrap();
    assert!(arena.is_descendant(deepest, root).unwrap());
    Visibility::Restricted(root)
        .validate_for(&arena, deepest)
        .unwrap();
}

#[test]
fn last_module_arc_releases_owned_storage_and_clone_preserves_authority() {
    let budget = ExpansionBudget::default();
    let baseline = budget.snapshot();
    let arena = ModuleArenaBuilder::for_catalog(
        budget.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &budget.clone(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap()
    .freeze()
    .unwrap();
    let root = arena.core_root();
    let held = budget.snapshot();
    assert!(held.retained_bytes > baseline.retained_bytes);
    assert_eq!(held.temporary_bytes, baseline.temporary_bytes);
    let clone = Arc::clone(&arena);
    assert!(Arc::ptr_eq(&clone, &arena));
    assert_eq!(budget.snapshot().retained_bytes, held.retained_bytes);
    drop(arena);
    clone.check(root).unwrap();
    assert_eq!(clone.core_root(), root);
    assert_eq!(budget.snapshot().retained_bytes, held.retained_bytes);
    drop(clone);
    let released = budget.snapshot();
    assert_eq!(released.retained_bytes, baseline.retained_bytes);
    assert_eq!(released.temporary_bytes, baseline.temporary_bytes);
    assert!(released.peak_bytes >= held.retained_bytes);
    assert!(released.counters[Counter::Work as usize] >= held.counters[Counter::Work as usize]);
}

#[test]
fn catalog_allocation_failure_and_raised_configuration_are_typed_not_fallback() {
    let zero = ExpansionBudget::new(ExpansionLimits {
        bytes: 0,
        ..ExpansionLimits::default()
    });
    let Err(ModuleError::Token(TokenError::Budget(failure))) = ModuleArenaBuilder::for_catalog(
        zero.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    ) else {
        panic!("zero bytes must fail as typed resource failure");
    };
    assert_eq!(failure.kind, BudgetFailureKind::Limit);
    assert_eq!(failure.resource, BudgetResource::Bytes);
    assert_eq!(zero.snapshot().retained_bytes, 0);
    assert_eq!(zero.snapshot().temporary_bytes, 0);
    let limits = ExpansionLimits {
        bytes: ExpansionLimits::default().bytes + 1,
        ..ExpansionLimits::default()
    };
    let raised = ExpansionBudget::new(limits);
    let Err(ModuleError::Token(TokenError::Budget(failure))) = ModuleArenaBuilder::for_catalog(
        raised.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    ) else {
        panic!("raised limit cannot create authority");
    };
    assert_eq!(failure.resource, BudgetResource::Limits);
    assert_eq!(raised.snapshot().retained_bytes, 0);
}

#[test]
fn shared_ledger_poison_blocks_existing_checked_module_and_privacy_operations() {
    let budget = ExpansionBudget::default();
    let arena = ModuleArenaBuilder::for_catalog(
        budget.clone(),
        justrust_engine::compiler::owner_path::standalone_issued(
            &budget.clone(),
            justrust_engine::compiler::owner_path::OwnerDomain::ModuleArena,
        )
        .unwrap(),
    )
    .unwrap()
    .freeze()
    .unwrap();
    let root = arena.core_root();
    let used = budget.snapshot().counters[Counter::Work as usize];
    let remaining = ExpansionLimits::default().counters[Counter::Work as usize] - used;
    let failure = budget
        .clone()
        .charge(Counter::Work, remaining + 1)
        .unwrap_err();
    assert_eq!(failure.resource, BudgetResource::Counter(Counter::Work));
    assert!(
        matches!(arena.check(root),Err(ModuleError::Token(TokenError::Budget(next))) if next==failure)
    );
    assert!(
        matches!(arena.parent(root),Err(ModuleError::Token(TokenError::Budget(next))) if next==failure)
    );
    assert!(
        matches!(arena.root_of(root),Err(ModuleError::Token(TokenError::Budget(next))) if next==failure)
    );
    assert!(
        matches!(arena.is_descendant(root,root),Err(ModuleError::Token(TokenError::Budget(next))) if next==failure)
    );
    assert!(
        matches!(Visibility::Public.accessible_from(&arena,root),Err(ModuleError::Token(TokenError::Budget(next))) if next==failure)
    );
    assert!(
        matches!(Visibility::Public.validate_for(&arena,root),Err(ModuleError::Token(TokenError::Budget(next))) if next==failure)
    );
    drop(arena);
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
}
