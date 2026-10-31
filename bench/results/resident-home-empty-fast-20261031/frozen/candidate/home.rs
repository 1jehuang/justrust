//! The crate an instance is compiled from the point of view of.
//!
//! An instance is asked for by a function of some crate, but what it compiles to depends only
//! on what its own name mentions: the function it instantiates and the types it is
//! instantiated with. Compiling it as the crate that asked would make `Vec<u8>::push` depend
//! on every file of the workspace crate that happened to use it, and give it one cache entry
//! per crate that does.

use base_db::{Crate, FileId};
use hir_def::{CallableDefId, DefWithBodyId, FunctionId, HasModule, Lookup};
use rustc_hash::{FxHashMap, FxHashSet};
use rustc_type_ir::{
    AliasTy, AliasTyKind, TypeSuperVisitable, TypeVisitable, TypeVisitor,
    inherent::{BoundExistentialPredicates as _, IntoKind, ParamEnv as _, SliceLike as _},
};
use salsa::plumbing::FromId as _;

use hir_ty::{
    ImplTraitId, InferBodyId,
    db::{HirDatabase, InternedClosureId, InternedCoroutineId},
    method_resolution::loaded_crates,
    next_solver::{DbInterner, Ty, TyKind},
    traits::{ParamEnvAndCrate, StoredParamEnvAndCrate},
};

use super::{Instance, InstanceDef};

/// The crates the name of an instance mentions.
struct Mentions<'db> {
    db: &'db dyn HirDatabase,
    crates: FxHashSet<Crate>,
    /// The bodies it has the code of or mentions a type of.
    bodies: Vec<DefWithBodyId>,
    /// Of those, the ones it mentions a coroutine of, whose layout is of the whole body.
    coroutines: Vec<DefWithBodyId>,
    /// Whether an `impl Trait` is looked through to the type its body gives it, and a
    /// projection left as it is written, rather than either told apart as `opaque`.
    reveal: bool,
    /// How many `impl Trait`s the type being looked at is inside of.
    revealing: u32,
    /// The bodies that give the `impl Trait`s it mentions their types, and those of the
    /// closures in those types: its name says none of what they wrote.
    hidden: Vec<DefWithBodyId>,
    /// It mentions something whose crate we can't tell.
    opaque: bool,
    /// The types looked into so far. The type of a future holds those of the futures it
    /// awaits, each of which holds its own, so the same few are reached along very many ways.
    seen: FxHashSet<Ty<'db>>,
}

impl<'db> Mentions<'db> {
    fn of(db: &'db dyn HirDatabase, instance: &Instance) -> Mentions<'db> {
        Mentions::looking(db, instance, false)
    }

    fn looking(db: &'db dyn HirDatabase, instance: &Instance, reveal: bool) -> Mentions<'db> {
        let mut mentions = Mentions {
            db,
            crates: FxHashSet::default(),
            bodies: Vec::new(),
            coroutines: Vec::new(),
            reveal,
            revealing: 0,
            hidden: Vec::new(),
            opaque: false,
            seen: FxHashSet::default(),
        };
        match instance.def {
            InstanceDef::Fn(func) | InstanceDef::VtableShim(func) => {
                mentions.crates.insert(func.krate(db));
                mentions.bodies.push(func.into());
            }
            InstanceDef::Closure(id) => {
                mentions.owner(InternedClosureId::from_id(id).loc(db).owner)
            }
            InstanceDef::Coroutine(id) | InstanceDef::CoroutineByMove(id) => {
                mentions.owner(InternedCoroutineId::from_id(id).loc(db).owner)
            }
            InstanceDef::DropGlue
            | InstanceDef::CallOnceShim
            | InstanceDef::FnPointerShim
            | InstanceDef::ReifyShim { .. } => {}
        }
        for arg in instance.args.as_ref().iter() {
            arg.visit_with(&mut mentions);
        }
        mentions
    }

    fn owner(&mut self, owner: InferBodyId<'_>) {
        let InferBodyId::DefWithBodyId(owner) = owner else {
            self.opaque = true;
            return;
        };
        let krate = match owner {
            DefWithBodyId::FunctionId(it) => it.krate(self.db),
            DefWithBodyId::ConstId(it) => it.krate(self.db),
            DefWithBodyId::StaticId(it) => it.krate(self.db),
            DefWithBodyId::VariantId(it) => it.krate(self.db),
        };
        self.crates.insert(krate);
        self.bodies.push(owner);
    }
}

/// The crates the name of an instance mentions, or `None` when one of them can't be told. Only
/// a crate that depends on all of them can have compiled a copy of it.
pub(super) fn mentioned_crates(
    db: &dyn HirDatabase,
    instance: &Instance,
) -> Option<FxHashSet<Crate>> {
    let mentions = Mentions::of(db, instance);
    (!mentions.opaque).then_some(mentions.crates)
}

/// The function the code of an instance is written in: its own, or the one a closure or an
/// `async` block is in. The symbol of every copy of the instance has its name.
pub(super) fn written_in(db: &dyn HirDatabase, instance: &Instance) -> Option<FunctionId> {
    let owner = match instance.def {
        InstanceDef::Fn(func) => return Some(func),
        InstanceDef::Closure(id) => InternedClosureId::from_id(id).loc(db).owner,
        InstanceDef::Coroutine(id) | InstanceDef::CoroutineByMove(id) => {
            InternedCoroutineId::from_id(id).loc(db).owner
        }
        InstanceDef::DropGlue
        | InstanceDef::CallOnceShim
        | InstanceDef::VtableShim(_)
        | InstanceDef::FnPointerShim
        | InstanceDef::ReifyShim { .. } => return None,
    };
    match owner {
        InferBodyId::DefWithBodyId(DefWithBodyId::FunctionId(func)) => Some(func),
        _ => None,
    }
}

/// The files with the bodies an instance has the code of or mentions a type of: those of its
/// function and of the closures, futures and functions it is instantiated with. `None` when
/// that can't be told.
pub fn body_files(db: &dyn HirDatabase, instance: &Instance) -> Option<Box<[FileId]>> {
    let mentions = Mentions::of(db, instance);
    if mentions.opaque {
        return None;
    }
    let mut files: Vec<FileId> = mentions.bodies.into_iter().map(|it| body_file(db, it)).collect();
    files.sort_unstable();
    files.dedup();
    Some(files.into())
}

/// The bodies the code of an instance depends on beyond its name: the one it is written in,
/// when it has one; those it mentions a coroutine of, whose layout is of the whole body; and
/// those that give the `impl Trait`s it mentions their types, with the closures and coroutines
/// in those. A closure or function its name mentions is in it whole, with what the closure
/// captures. `None` when that can't be told.
pub(super) fn bodies_of(
    db: &dyn HirDatabase,
    instance: &Instance,
) -> Option<(Option<DefWithBodyId>, Vec<DefWithBodyId>)> {
    let mut mentions = Mentions::looking(db, instance, true);
    if mentions.opaque {
        return None;
    }
    mentions.coroutines.append(&mut mentions.hidden);
    let written = match instance.def {
        InstanceDef::Fn(_)
        | InstanceDef::VtableShim(_)
        | InstanceDef::Closure(_)
        | InstanceDef::Coroutine(_)
        | InstanceDef::CoroutineByMove(_) => mentions.bodies.first().copied(),
        _ => None,
    };
    Some((written, mentions.coroutines))
}

/// The file a body is written in, which for one a macro makes is where the macro is called.
pub fn body_file(db: &dyn HirDatabase, body: DefWithBodyId) -> FileId {
    let file = match body {
        DefWithBodyId::FunctionId(it) => it.lookup(db).id.file_id,
        DefWithBodyId::ConstId(it) => it.lookup(db).id.file_id,
        DefWithBodyId::StaticId(it) => it.lookup(db).id.file_id,
        DefWithBodyId::VariantId(it) => it.lookup(db).parent.lookup(db).id.file_id,
    };
    file.original_file_respecting_includes(db).file_id(db)
}

impl<'db> TypeVisitor<DbInterner<'db>> for Mentions<'db> {
    type Result = ();

    fn visit_ty(&mut self, ty: Ty<'db>) {
        if !self.seen.insert(ty) {
            return;
        }
        match ty.kind() {
            TyKind::Adt(adt, _) => {
                self.crates.insert(adt.def_id().module(self.db).krate(self.db));
            }
            TyKind::FnDef(def, _) => {
                let krate = match def.0 {
                    CallableDefId::FunctionId(it) => {
                        self.bodies.push(it.into());
                        it.krate(self.db)
                    }
                    CallableDefId::StructId(it) => it.krate(self.db),
                    CallableDefId::EnumVariantId(it) => it.krate(self.db),
                };
                self.crates.insert(krate);
            }
            TyKind::Closure(id, _) => {
                let known = self.bodies.len();
                self.owner(id.0.loc(self.db).owner);
                if self.revealing > 0 {
                    self.hidden.extend(self.bodies.get(known).copied());
                }
            }
            TyKind::CoroutineClosure(id, _) => {
                self.owner(id.0.loc(self.db).owner);
                self.coroutines.extend(self.bodies.last().copied());
            }
            TyKind::Coroutine(id, _) => {
                self.owner(id.0.loc(self.db).owner);
                self.coroutines.extend(self.bodies.last().copied());
            }
            TyKind::Dynamic(bounds, _) => {
                if let Some(principal) = bounds.principal() {
                    self.crates.insert(principal.skip_binder().def_id.0.krate(self.db));
                }
            }
            TyKind::Alias(AliasTy { kind: AliasTyKind::Opaque { def_id }, args, .. })
                if self.reveal =>
            {
                let ImplTraitId::ReturnTypeImplTrait(of, _) = def_id.0.loc(self.db) else {
                    self.opaque = true;
                    return;
                };
                self.hidden.push(of.into());
                let interner = DbInterner::new_no_crate(self.db);
                let hidden = rustc_type_ir::Interner::type_of(interner, def_id.into())
                    .instantiate(interner, args)
                    .skip_norm_wip();
                if matches!(hidden.kind(), TyKind::Error(_)) {
                    self.opaque = true;
                    return;
                }
                self.revealing += 1;
                self.visit_ty(hidden);
                self.revealing -= 1;
            }
            // What no `impl` gave a type is the type its name spells.
            TyKind::Alias(AliasTy { kind: AliasTyKind::Projection { .. }, .. }) if self.reveal => {}
            TyKind::Alias(..) | TyKind::Foreign(..) => self.opaque = true,
            _ => {}
        }
        ty.super_visit_with(self)
    }
}

/// Picks the crate each instance is compiled as.
#[derive(Default)]
pub(super) struct Homes {
    /// What each crate can see: itself and the crates whose impls it knows.
    sees: FxHashMap<Crate, FxHashSet<Crate>>,
}

/// Process-latched, exact opt-in. The dynamic NO_HOME guard stays authoritative.
fn empty_fast_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RA_JIT_HOME_EMPTY_FAST").is_some_and(|v| v == "1"))
}

/// Emit only after entering the specialization. Thread-local initialization avoids a
/// process-wide atomic check on subsequent calls from the same thread.
fn mark_empty_fast_branch() {
    thread_local! {
        static MARKED: () = {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                use std::io::Write as _;
                let _ = std::io::stderr().lock().write_all(b"ra-jit: home-empty-fast-v68 branch\n");
            });
        };
    }
    MARKED.with(|()| {});
}

impl Homes {
    /// The environment to compile `instance` in when a function compiled in `asked` asks for
    /// it: that of the one crate it mentions that sees all the others, which every crate
    /// asking for the instance agrees on. Without such a crate, the asking one's.
    pub(super) fn home(
        &mut self,
        db: &dyn HirDatabase,
        instance: &Instance,
        asked: &StoredParamEnvAndCrate,
    ) -> StoredParamEnvAndCrate {
        if std::env::var_os("RA_JIT_NO_HOME").is_some()
            || !asked.param_env(db).caller_bounds().is_empty()
        {
            return asked.clone();
        }
        if empty_fast_enabled() && instance.args.as_ref().is_empty() {
            if let InstanceDef::Fn(func) = instance.def {
                mark_empty_fast_branch();
                let krate = func.krate(db);
                let sees = self
                    .sees
                    .entry(krate)
                    .or_insert_with(|| loaded_crates(db, krate).iter().copied().collect());
                return if sees.contains(&krate) {
                    ParamEnvAndCrate { krate, ..asked.as_ref(db) }.store()
                } else {
                    asked.clone()
                };
            }
        }
        let mentions = Mentions::of(db, instance);
        if mentions.opaque {
            return asked.clone();
        }
        let home = mentions.crates.iter().copied().find(|&krate| {
            let sees = self
                .sees
                .entry(krate)
                .or_insert_with(|| loaded_crates(db, krate).iter().copied().collect());
            mentions.crates.iter().all(|it| sees.contains(it))
        });
        match home {
            Some(krate) => ParamEnvAndCrate { krate, ..asked.as_ref(db) }.store(),
            None => asked.clone(),
        }
    }
}

#[cfg(test)]
mod empty_fast_tests {
    use super::*;
    use base_db::all_crates;
    use hir_def::{ModuleDefId, nameres::crate_def_map, signatures::FunctionSignature};
    use hir_expand::change::ChangeWithProcMacros;
    use hir_ty::{
        InferenceResult,
        next_solver::{GenericArgs, ParamEnv},
    };
    use ide_db::RootDatabase;
    use salsa::plumbing::AsId as _;
    use test_fixture::WithFixture;

    // Frozen pre-v68 algorithm, deliberately independent of Homes::home and its flag.
    // Its own cache also makes changed query dependencies observable independently.
    #[derive(Default)]
    struct OriginalHomes {
        sees: FxHashMap<Crate, FxHashSet<Crate>>,
    }

    impl OriginalHomes {
        fn home(
            &mut self,
            db: &dyn HirDatabase,
            instance: &Instance,
            asked: &StoredParamEnvAndCrate,
        ) -> StoredParamEnvAndCrate {
            if std::env::var_os("RA_JIT_NO_HOME").is_some()
                || !asked.param_env(db).caller_bounds().is_empty()
            {
                return asked.clone();
            }
            let mentions = Mentions::of(db, instance);
            if mentions.opaque {
                return asked.clone();
            }
            let home = mentions.crates.iter().copied().find(|&krate| {
                let sees = self
                    .sees
                    .entry(krate)
                    .or_insert_with(|| loaded_crates(db, krate).iter().copied().collect());
                mentions.crates.iter().all(|it| sees.contains(it))
            });
            match home {
                Some(krate) => ParamEnvAndCrate { krate, ..asked.as_ref(db) }.store(),
                None => asked.clone(),
            }
        }
    }

    fn function(db: &RootDatabase, name: &str) -> FunctionId {
        let mut found = Vec::new();
        for &krate in all_crates(db).iter() {
            for (_, module) in crate_def_map(db, krate).modules(db) {
                for def in module.scope(db).declarations() {
                    if let ModuleDefId::FunctionId(id) = def {
                        if FunctionSignature::of(db, id).name.as_str() == name {
                            found.push(id);
                        }
                    }
                }
            }
        }
        assert_eq!(found.len(), 1, "fixture function must be unique: {name}");
        found[0]
    }

    fn instance(func: FunctionId) -> Instance {
        Instance {
            def: InstanceDef::Fn(func),
            args: GenericArgs::new_from_slice(&[]).store(),
            checked: false,
        }
    }

    fn asked(db: &RootDatabase, func: FunctionId) -> StoredParamEnvAndCrate {
        ParamEnvAndCrate { param_env: ParamEnv::empty(), krate: func.krate(db) }.store()
    }

    fn compare(
        db: &RootDatabase,
        candidate: &mut Homes,
        original: &mut OriginalHomes,
        instance: &Instance,
        asked: &StoredParamEnvAndCrate,
    ) -> StoredParamEnvAndCrate {
        let expected = original.home(db, instance, asked);
        let actual = candidate.home(db, instance, asked);
        // Compare the entire stored environment, not just the chosen crate.
        assert_eq!(actual, expected);
        assert_eq!(actual.as_ref(db), expected.as_ref(db));
        assert_eq!(candidate.sees, original.sees);
        actual
    }

    fn fixture(cfg: &str) -> String {
        format!(
            r#"
//- /app.rs crate:app deps:dep cfg:{cfg}
fn request() {{ dep::target(); }}
fn local() {{}}
fn closure_owner() {{ let closure = || 7u32; }}
trait Bound {{}}
impl Bound for u32 {{}}
fn opaque() -> impl Bound {{ 7u32 }}
fn bounded<T: Bound>() {{}}
fn generic<T>() {{}}
//- /dep.rs crate:dep deps:signature cfg:{cfg}
pub fn target() -> signature::Token {{ signature::Token }}
#[cfg(selected)] pub fn selected_target() {{}}
#[cfg(not(selected))] pub fn other_target() {{}}
//- /signature.rs crate:signature
pub struct Token;
"#
        )
    }

    #[test]
    fn v68_real_database_same_repeated_cross_crate_and_cfg() {
        assert!(std::env::var_os("RA_JIT_NO_HOME").is_none(), "run guard test separately");
        for cfg in ["selected", "other"] {
            let db = RootDatabase::with_files(&fixture(cfg));
            let request = function(&db, "request");
            let local = function(&db, "local");
            let target = function(&db, "target");
            let configured =
                function(&db, if cfg == "selected" { "selected_target" } else { "other_target" });
            let mut candidate = Homes::default();
            let mut original = OriginalHomes::default();
            for caller in [request, target] {
                for callee in [local, target, configured] {
                    for _ in 0..3 {
                        compare(
                            &db,
                            &mut candidate,
                            &mut original,
                            &instance(callee),
                            &asked(&db, caller),
                        );
                    }
                }
            }
            // Exercise the false membership outcome explicitly. Never assume self is loaded.
            let krate = target.krate(&db);
            candidate.sees.insert(krate, FxHashSet::default());
            original.sees.insert(krate, FxHashSet::default());
            let env = asked(&db, request);
            assert_eq!(compare(&db, &mut candidate, &mut original, &instance(target), &env), env);
        }
    }

    #[test]
    fn v68_real_database_signature_and_dependency_revisions() {
        assert!(std::env::var_os("RA_JIT_NO_HOME").is_none(), "run guard test separately");
        let (mut db, files) = RootDatabase::with_many_files(&fixture("selected"));
        let request = function(&db, "request");
        let target = function(&db, "target");
        let env = asked(&db, request);
        let mut candidate = Homes::default();
        let mut original = OriginalHomes::default();
        compare(&db, &mut candidate, &mut original, &instance(target), &env);
        for source in [
            "pub fn target() -> u32 { 1 }",
            "pub fn target() -> signature::Token { signature::Token }",
        ] {
            let mut change = ChangeWithProcMacros::default();
            change.change_file(files[1].file_id(&db), Some(source.into()));
            change.apply(&mut db);
            let target = function(&db, "target");
            // Existing per-Homes cache semantics are retained across database revisions.
            compare(&db, &mut candidate, &mut original, &instance(target), &env);
            // Also query independently refreshed loaded_crates dependencies.
            compare(
                &db,
                &mut Homes::default(),
                &mut OriginalHomes::default(),
                &instance(target),
                &env,
            );
        }
    }

    #[test]
    fn v68_real_database_generic_closure_shim_and_caller_bounds_fallback() {
        assert!(std::env::var_os("RA_JIT_NO_HOME").is_none(), "run guard test separately");
        let db = RootDatabase::with_files(&fixture("selected"));
        let request = function(&db, "request");
        let target = function(&db, "target");
        let generic = function(&db, "generic");
        let bounded = function(&db, "bounded");
        let owner = function(&db, "closure_owner");
        let env = asked(&db, request);
        let mut candidate = Homes::default();
        let mut original = OriginalHomes::default();
        let mut nonempty = instance(generic);
        nonempty.args =
            GenericArgs::new_from_slice(&[Ty::new_without_interner(TyKind::Bool).into()]).store();
        assert!(!nonempty.args.as_ref().is_empty());
        compare(&db, &mut candidate, &mut original, &nonempty, &env);
        let opaque = function(&db, "opaque");
        let opaque_ty =
            db.callable_item_signature(opaque.into()).skip_binder().skip_binder().output();
        assert!(
            matches!(opaque_ty.kind(), TyKind::Alias(..)),
            "fixture must expose a real opaque return type"
        );
        let opaque_instance = Instance {
            args: GenericArgs::new_from_slice(&[opaque_ty.into()]).store(),
            ..instance(generic)
        };
        assert!(Mentions::of(&db, &opaque_instance).opaque);
        assert_eq!(compare(&db, &mut candidate, &mut original, &opaque_instance, &env), env);
        let closure = InferenceResult::of(&db, owner)
            .expression_types()
            .find_map(|(_, ty)| match ty.kind() {
                TyKind::Closure(id, args) => Some(Instance {
                    def: InstanceDef::Closure(id.0.as_id()),
                    args: args.store(),
                    checked: false,
                }),
                _ => None,
            })
            .expect("fixture must infer a real closure");
        compare(&db, &mut candidate, &mut original, &closure, &env);
        for def in [
            InstanceDef::VtableShim(target),
            InstanceDef::DropGlue,
            InstanceDef::CallOnceShim,
            InstanceDef::FnPointerShim,
            InstanceDef::ReifyShim { receiver: false },
        ] {
            compare(
                &db,
                &mut candidate,
                &mut original,
                &Instance { def, ..instance(target) },
                &env,
            );
        }
        let bounded_env = ParamEnvAndCrate {
            param_env: db.trait_environment(bounded.into()),
            krate: request.krate(&db),
        }
        .store();
        assert!(!bounded_env.param_env(&db).caller_bounds().is_empty());
        let mut candidate = Homes::default();
        let mut original = OriginalHomes::default();
        assert_eq!(
            compare(&db, &mut candidate, &mut original, &instance(target), &bounded_env),
            bounded_env
        );
        assert!(candidate.sees.is_empty(), "caller bounds must guard before specialization");
    }

    #[test]
    #[ignore = "run in a fresh process with RA_JIT_NO_HOME=1, independently of regular fixtures"]
    fn v68_real_database_no_home_guard() {
        assert!(std::env::var_os("RA_JIT_NO_HOME").is_some());
        let db = RootDatabase::with_files(&fixture("selected"));
        let target = function(&db, "target");
        let env = asked(&db, function(&db, "request"));
        let mut candidate = Homes::default();
        let mut original = OriginalHomes::default();
        assert_eq!(compare(&db, &mut candidate, &mut original, &instance(target), &env), env);
        assert!(candidate.sees.is_empty());
    }
}
