//! Handling of opaque types, detection of defining scope and hidden type.

use base_db::Crate;
use hir_def::{
    AssocItemId, AssocItemLoc, DefWithBodyId, FunctionId, HasModule, ItemContainerId, ModuleDefId,
    TypeAliasId,
    attrs::AttrFlags,
    expr_store::Body,
    nameres::{DefMap, crate_def_map},
    resolver::HasResolver,
    signatures::ImplSignature,
};
use hir_expand::name::Name;
use la_arena::ArenaMap;
use rustc_type_ir::inherent::Ty as _;
use syntax::{TextSize, ast};

use crate::{
    ImplTraitId, InferBodyId, InferenceResult,
    db::{HirDatabase, InternedOpaqueTyId},
    lower::{ImplTraitIdx, SelfAndAssocBounds},
    next_solver::{
        DbInterner, ErrorGuaranteed, SolverDefId, StoredEarlyBinder, StoredTy, Ty, TypingMode,
        infer::{DbInternerInferExt, traits::ObligationCause},
        obligation_ctxt::ObligationCtxt,
    },
};

pub(crate) fn opaque_types_defined_by<'db>(
    db: &'db dyn HirDatabase,
    def_id: InferBodyId<'_>,
    result: &mut Vec<SolverDefId<'db>>,
) {
    if let Some(func) = def_id.as_function() {
        // A function may define its own RPITs.
        extend_with_opaques(
            db,
            SelfAndAssocBounds::return_type_impl_traits(db, func),
            |opaque_idx| ImplTraitId::ReturnTypeImplTrait(func, opaque_idx),
            result,
        );
    }

    let extend_with_taits = |type_alias| {
        extend_with_opaques(
            db,
            SelfAndAssocBounds::type_alias_impl_traits(db, type_alias),
            |opaque_idx| ImplTraitId::TypeAliasImplTrait(type_alias, opaque_idx),
            result,
        );
    };

    // Collect opaques from assoc items.
    let extend_with_atpit_from_assoc_items = |assoc_items: &[(Name, AssocItemId)]| {
        assoc_items
            .iter()
            .filter_map(|&(_, assoc_id)| match assoc_id {
                AssocItemId::TypeAliasId(it) => Some(it),
                AssocItemId::FunctionId(_) | AssocItemId::ConstId(_) => None,
            })
            .for_each(extend_with_taits);
    };
    let extend_with_atpit_from_container = |container| match container {
        ItemContainerId::ImplId(impl_id)
            if ImplSignature::of(db, impl_id).target_trait.is_some() =>
        {
            extend_with_atpit_from_assoc_items(&impl_id.impl_items(db).items);
        }
        ItemContainerId::TraitId(trait_id) => {
            extend_with_atpit_from_assoc_items(&trait_id.trait_items(db).items);
        }
        _ => {}
    };
    match def_id {
        InferBodyId::DefWithBodyId(DefWithBodyId::ConstId(id)) => {
            extend_with_atpit_from_container(id.loc(db).container)
        }
        InferBodyId::DefWithBodyId(DefWithBodyId::FunctionId(id)) => {
            extend_with_atpit_from_container(id.loc(db).container)
        }
        InferBodyId::DefWithBodyId(DefWithBodyId::StaticId(_))
        | InferBodyId::DefWithBodyId(DefWithBodyId::VariantId(_))
        | InferBodyId::AnonConstId(_) => {}
    }

    if let Some(func) = def_id.as_function() {
        for type_alias in defined_type_aliases(db, func) {
            extend_with_opaques(
                db,
                SelfAndAssocBounds::type_alias_impl_traits(db, type_alias),
                |opaque_idx| ImplTraitId::TypeAliasImplTrait(type_alias, opaque_idx),
                result,
            );
        }
    }

    fn extend_with_opaques<'db>(
        db: &'db dyn HirDatabase,
        opaques: &[SelfAndAssocBounds],
        mut make_impl_trait: impl FnMut(ImplTraitIdx) -> ImplTraitId,
        result: &mut Vec<SolverDefId<'db>>,
    ) {
        for (opaque_idx, _) in opaques.iter().enumerate() {
            let opaque_id = InternedOpaqueTyId::new(
                db,
                make_impl_trait(ImplTraitIdx::from_raw(la_arena::RawIdx::from_u32(
                    opaque_idx as u32,
                ))),
            );
            result.push(opaque_id.into());
        }
    }
}

// These are firewall queries to prevent drawing dependencies between infers:

#[salsa::tracked(returns(ref))]
pub(crate) fn rpit_hidden_types(
    db: &dyn HirDatabase,
    function: FunctionId,
) -> ArenaMap<ImplTraitIdx, StoredEarlyBinder<StoredTy>> {
    let infer = InferenceResult::of(db, DefWithBodyId::from(function));
    let mut result = ArenaMap::new();
    for (opaque, hidden_type) in infer.return_position_impl_trait_types(db) {
        result.insert(opaque, StoredEarlyBinder::bind(hidden_type.store()));
    }
    result.shrink_to_fit();
    result
}

#[salsa::tracked(returns(ref))]
pub(crate) fn tait_hidden_types(
    db: &dyn HirDatabase,
    type_alias: TypeAliasId,
) -> ArenaMap<ImplTraitIdx, StoredEarlyBinder<StoredTy>> {
    // Call this first, to not perform redundant work if there are no TAITs.
    let taits_count = SelfAndAssocBounds::type_alias_impl_traits(db, type_alias).len();

    let loc = type_alias.loc(db);
    let module = loc.module(db);
    let interner = DbInterner::new_with(db, module.krate(db));
    let infcx = interner.infer_ctxt().build(TypingMode::non_body_analysis());
    let mut ocx = ObligationCtxt::new(&infcx);
    let cause = ObligationCause::dummy();
    let param_env = db.trait_environment(type_alias.into());

    let defining_bodies = tait_defining_bodies(db, loc);

    let mut result = ArenaMap::with_capacity(taits_count);
    for defining_body in defining_bodies {
        let infer = InferenceResult::of(db, defining_body);
        for (&opaque, hidden_type) in &infer.type_of_opaque {
            let ImplTraitId::TypeAliasImplTrait(opaque_owner, opaque_idx) = opaque.loc(db) else {
                continue;
            };
            if opaque_owner != type_alias {
                continue;
            }
            // In the presence of errors, we attempt to create a unified type from all
            // types. rustc doesn't do that, but this should improve the experience.
            let hidden_type = infcx.insert_type_vars(hidden_type.as_ref());
            match result.entry(opaque_idx) {
                la_arena::Entry::Vacant(entry) => {
                    entry.insert(StoredEarlyBinder::bind(hidden_type.store()));
                }
                la_arena::Entry::Occupied(entry) => {
                    _ = ocx.eq(
                        &cause,
                        param_env,
                        entry.get().get().instantiate_identity().skip_norm_wip(),
                        hidden_type,
                    );
                }
            }
        }
    }

    _ = ocx.try_evaluate_obligations();

    // Fill missing entries.
    for idx in 0..taits_count {
        let idx = la_arena::Idx::from_raw(la_arena::RawIdx::from_u32(idx as u32));
        match result.entry(idx) {
            la_arena::Entry::Vacant(entry) => {
                entry.insert(StoredEarlyBinder::bind(
                    Ty::new_error(interner, ErrorGuaranteed).store(),
                ));
            }
            la_arena::Entry::Occupied(mut entry) => {
                let hidden_type = entry.get().get().skip_binder();
                let hidden_type =
                    infcx.resolve_vars_if_possible(hidden_type).replace_infer_with_error(interner);
                *entry.get_mut() = StoredEarlyBinder::bind(hidden_type.store());
            }
        }
    }

    result
}

fn tait_defining_bodies(
    db: &dyn HirDatabase,
    loc: &AssocItemLoc<ast::TypeAlias>,
) -> Vec<DefWithBodyId> {
    let from_assoc_items = |assoc_items: &[(Name, AssocItemId)]| {
        // Associated Type Position Impl Trait.
        assoc_items
            .iter()
            .filter_map(|&(_, assoc_id)| match assoc_id {
                AssocItemId::FunctionId(it) => Some(it.into()),
                AssocItemId::ConstId(it) => Some(it.into()),
                AssocItemId::TypeAliasId(_) => None,
            })
            .collect()
    };
    match loc.container {
        ItemContainerId::ImplId(impl_id)
            if ImplSignature::of(db, impl_id).target_trait.is_some() =>
        {
            return from_assoc_items(&impl_id.impl_items(db).items);
        }
        ItemContainerId::TraitId(trait_id) => {
            return from_assoc_items(&trait_id.trait_items(db).items);
        }
        _ => {}
    }

    define_opaque_functions(db, loc.module(db).krate(db))
        .iter()
        .filter(|&&func| defined_type_aliases(db, func).any(|alias| alias.loc(db) == loc))
        .map(|&func| func.into())
        .collect()
}

/// The type aliases `func` names in its `#[define_opaque(..)]`.
fn defined_type_aliases(
    db: &dyn HirDatabase,
    func: FunctionId,
) -> impl Iterator<Item = TypeAliasId> {
    let paths = AttrFlags::define_opaque(db, func);
    let resolver = (!paths.is_empty()).then(|| func.resolver(db));
    paths.iter().filter_map(move |path| {
        match resolver.as_ref()?.resolve_module_path_in_items(db, path).take_types()? {
            ModuleDefId::TypeAliasId(it) => Some(it),
            _ => None,
        }
    })
}

/// Every function of the crate with a `#[define_opaque(..)]`: in a module, in an impl, and in
/// a block of a body, as std's `sys::paths::unix::split_paths_ref` has the function that
/// defines `SplitPathsRef` inside it. Of a crate that is not local, a body is lowered for its
/// blocks only when its text says `define_opaque`, so one written by a macro there is not found
/// and its opaque type stays an error, which no function using it compiles with.
#[salsa::tracked(returns(deref))]
fn define_opaque_functions(db: &dyn HirDatabase, krate: Crate) -> Box<[FunctionId]> {
    let local = krate.data(db).origin.is_local();
    let mut result = Vec::new();
    collect(db, crate_def_map(db, krate), local, &mut result);
    return result.into_boxed_slice();

    fn collect(db: &dyn HirDatabase, def_map: &DefMap, local: bool, result: &mut Vec<FunctionId>) {
        for (_, module) in def_map.modules(db) {
            let scope = module.scope(db);
            let mut holders: Vec<DefWithBodyId> = Vec::new();
            for def in scope.declarations() {
                match def {
                    ModuleDefId::FunctionId(it) => holders.push(it.into()),
                    ModuleDefId::ConstId(it) => holders.push(it.into()),
                    ModuleDefId::StaticId(it) => holders.push(it.into()),
                    _ => {}
                }
            }
            for impl_id in scope.impls() {
                for &(_, item) in &impl_id.impl_items(db).items {
                    match item {
                        AssocItemId::FunctionId(it) => holders.push(it.into()),
                        AssocItemId::ConstId(it) => holders.push(it.into()),
                        AssocItemId::TypeAliasId(_) => {}
                    }
                }
            }
            for owner in holders {
                if let DefWithBodyId::FunctionId(func) = owner
                    && !AttrFlags::define_opaque(db, func).is_empty()
                {
                    result.push(func);
                }
                if local || mentions_define_opaque(db, owner) {
                    for (_, block) in Body::of(db, owner).blocks(db) {
                        collect(db, block, local, result);
                    }
                }
            }
        }
    }
}

/// Whether the text of `owner` says `define_opaque`, in a file of its own: where its blocks
/// may have a function with the attribute.
fn mentions_define_opaque(db: &dyn HirDatabase, owner: DefWithBodyId) -> bool {
    let (file_id, range) = match owner {
        DefWithBodyId::FunctionId(it) => {
            let id = it.loc(db).id;
            (id.file_id, id.file_id.ast_id_map(db).get(id.value).text_range())
        }
        DefWithBodyId::ConstId(it) => {
            let id = it.loc(db).id;
            (id.file_id, id.file_id.ast_id_map(db).get(id.value).text_range())
        }
        DefWithBodyId::StaticId(it) => {
            let id = it.loc(db).id;
            (id.file_id, id.file_id.ast_id_map(db).get(id.value).text_range())
        }
        DefWithBodyId::VariantId(_) => return false,
    };
    let Some(file) = file_id.file_id() else { return false };
    define_opaque_at(db, file).iter().any(|&at| range.contains(at))
}

/// Where the text of `file` says `define_opaque`.
#[salsa::tracked(returns(deref))]
fn define_opaque_at(db: &dyn HirDatabase, file: base_db::EditionedFileId) -> Box<[TextSize]> {
    let text = db.file_text(file.file_id(db)).text(db);
    let found = text.match_indices("define_opaque").map(|(at, _)| TextSize::from(at as u32));
    found.collect()
}
