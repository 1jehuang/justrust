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

/// Keep the visitor's set iteration order: `Homes::home` picks the first eligible crate.
/// Sorting this result, or rebuilding a set before home selection, can change that choice.
fn direct_home_crates(db: &dyn HirDatabase, instance: &Instance) -> Option<Vec<Crate>> {
    let mentions = Mentions::of(db, instance);
    (!mentions.opaque).then(|| mentions.crates.into_iter().collect())
}

/// Native Salsa memoization, keyed by the *whole* instance, including its stored arguments.
/// All database reads remain inside this provider, so Salsa records and validates the same
/// dependencies as the direct visitor. There is deliberately no process-global value cache.
#[salsa::tracked(returns(ref))]
fn native_home_crates(db: &dyn HirDatabase, instance: Instance) -> Option<Vec<Crate>> {
    #[cfg(test)]
    HOME_PROVIDER_EXECUTIONS.with(|count| count.set(count.get() + 1));
    if home_memo_active() {
        static ENTERED: std::sync::Once = std::sync::Once::new();
        ENTERED.call_once(|| {
            use std::io::Write as _;
            let _ = writeln!(
                std::io::stderr().lock(),
                "ra_jit: native-home-query-v66 QUERY ENTERED (not workload cache-hit evidence)"
            );
        });
    }
    direct_home_crates(db, &instance)
}

fn home_crates<'db>(db: &'db dyn HirDatabase, instance: &Instance) -> Option<HomeCrates<'db>> {
    home_crates_with_memo(db, instance, home_memo_active())
}

fn home_memo_active() -> bool {
    // This experimental mode is fixed at first use in each process. In particular the
    // disabled hot path must not scan the process environment for every instance.
    static MEMO: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| home_memo_enabled(std::env::var_os("RA_JIT_HOME_MEMO")));
    *MEMO
}

fn home_memo_enabled(value: Option<std::ffi::OsString>) -> bool {
    value.as_deref() == Some(std::ffi::OsStr::new("1"))
}

// An explicit switch lets fixtures exercise the actual wrapper without mutating the process
// environment in parallel tests. Production passes only the exact environment check above.
fn home_crates_with_memo<'db>(
    db: &'db dyn HirDatabase,
    instance: &Instance,
    memo: bool,
) -> Option<HomeCrates<'db>> {
    if memo && !instance.args.as_ref().is_empty() {
        native_home_crates(db, instance.clone()).as_deref().map(HomeCrates::Memo)
    } else {
        let mentions = Mentions::of(db, instance);
        (!mentions.opaque).then_some(HomeCrates::Direct(mentions.crates))
    }
}

enum HomeCrates<'db> {
    Direct(FxHashSet<Crate>),
    Memo(&'db [Crate]),
}

impl HomeCrates<'_> {
    fn iter(&self) -> impl Iterator<Item = &Crate> {
        match self {
            Self::Direct(crates) => either::Either::Left(crates.iter()),
            Self::Memo(crates) => either::Either::Right(crates.iter()),
        }
    }

    fn into_set(self) -> FxHashSet<Crate> {
        match self {
            Self::Direct(crates) => crates,
            Self::Memo(crates) => crates.iter().copied().collect(),
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static HOME_PROVIDER_EXECUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
    home_crates(db, instance).map(HomeCrates::into_set)
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
        let Some(crates) = home_crates(db, instance) else { return asked.clone() };
        let home = crates.iter().copied().find(|&krate| {
            let sees = self
                .sees
                .entry(krate)
                .or_insert_with(|| loaded_crates(db, krate).iter().copied().collect());
            crates.iter().all(|it| sees.contains(it))
        });
        match home {
            Some(krate) => ParamEnvAndCrate { krate, ..asked.as_ref(db) }.store(),
            None => asked.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base_db::{SourceDatabase, all_crates};
    use hir_def::{ModuleDefId, nameres::crate_def_map};
    use hir_expand::name::Name;
    use hir_ty::{InferenceResult, next_solver::GenericArgs};
    use ide_db::RootDatabase;
    use test_fixture::WithFixture;

    fn executions() -> usize {
        HOME_PROVIDER_EXECUTIONS.with(|count| count.replace(0))
    }

    fn function(db: &dyn HirDatabase, name: &str) -> FunctionId {
        for &krate in all_crates(db).iter() {
            let map = crate_def_map(db, krate);
            let scope = map.module(db, map.root_module_id()).scope(db);
            if let Some(ModuleDefId::FunctionId(function)) =
                scope.get(&Name::new_root(name)).take_values()
            {
                return function;
            }
        }
        panic!("fixture function {name} not found")
    }

    fn return_instance(
        db: &dyn HirDatabase,
        generic: FunctionId,
        producer: FunctionId,
    ) -> Instance {
        let ty = db.callable_item_signature(producer.into()).skip_binder().skip_binder().output();
        Instance {
            def: InstanceDef::Fn(generic),
            args: GenericArgs::new_from_slice(&[ty.into()]).store(),
            checked: false,
        }
    }

    // This oracle deliberately calls the original visitor, not the query or wrapper.
    fn oracle(db: &dyn HirDatabase, instance: &Instance) -> Option<Vec<Crate>> {
        let mentions = Mentions::of(db, instance);
        if mentions.opaque { None } else { Some(mentions.crates.iter().copied().collect()) }
    }

    fn wrapped(db: &dyn HirDatabase, instance: &Instance, memo: bool) -> Option<Vec<Crate>> {
        home_crates_with_memo(db, instance, memo).map(|crates| crates.iter().copied().collect())
    }

    #[salsa::tracked(returns(ref))]
    fn signature_result(
        db: &dyn HirDatabase,
        generic: FunctionId,
        producer: FunctionId,
    ) -> Option<Vec<Crate>> {
        wrapped(db, &return_instance(db, generic, producer), true)
    }

    fn closure_instance(db: &dyn HirDatabase, generic: FunctionId) -> Instance {
        for name in ["owner_a", "owner_b"] {
            let owner = function(db, name);
            let inferred = InferenceResult::of(db, DefWithBodyId::from(owner));
            for (_, ty) in inferred.type_of_expr.iter() {
                let ty = ty.as_ref();
                if matches!(ty.kind(), TyKind::Closure(..)) {
                    return Instance {
                        def: InstanceDef::Fn(generic),
                        args: GenericArgs::new_from_slice(&[ty.into()]).store(),
                        checked: false,
                    };
                }
            }
        }
        panic!("fixture has no inferred closure")
    }

    #[salsa::tracked(returns(ref))]
    fn closure_result(db: &dyn HirDatabase, generic: FunctionId) -> Option<Vec<Crate>> {
        wrapped(db, &closure_instance(db, generic), true)
    }

    #[test]
    fn native_home_exact_opt_in() {
        assert!(!home_memo_enabled(None));
        for value in ["", "0", "true", "01", "1 ", " 1"] {
            assert!(!home_memo_enabled(Some(value.into())));
        }
        assert!(home_memo_enabled(Some("1".into())));
    }

    #[test]
    fn native_home_real_db_cold_repeat_full_key_and_empty_bypass() {
        let (db, _) =
            RootDatabase::with_single_file("fn generic<T>() {}\nfn producer() -> u32 { 0 }");
        let generic = function(&db, "generic");
        let mut instance = return_instance(&db, generic, function(&db, "producer"));
        let expected = oracle(&db, &instance);
        executions();
        assert_eq!(wrapped(&db, &instance, false), expected);
        assert_eq!(executions(), 0, "default-off wrapper must not execute the provider");
        assert_eq!(wrapped(&db, &instance, true), expected);
        assert_eq!(executions(), 1, "one real provider execution on a cold key");
        for _ in 0..8 {
            assert_eq!(wrapped(&db, &instance, true), expected);
        }
        assert_eq!(executions(), 0, "same-revision hits must not execute the provider");
        instance.checked = true;
        assert_eq!(wrapped(&db, &instance, true), oracle(&db, &instance));
        assert_eq!(executions(), 1, "checked is part of the full Instance key");
        instance.args = GenericArgs::new_from_slice(&[]).store();
        assert_eq!(wrapped(&db, &instance, true), oracle(&db, &instance));
        assert_eq!(executions(), 0, "empty arguments retain the direct visitor path");
    }

    #[test]
    fn native_home_real_db_signature_and_dependency_revisions() {
        let (mut db, files) = RootDatabase::with_many_files(
            r#"
//- /main.rs crate:main deps:dep
fn generic<T>() {}
fn producer() -> dep::Item { loop {} }
fn producer_again() -> dep::Item { loop {} }
//- /dep.rs crate:dep deps:left,right
pub use left::Item;
//- /left.rs crate:left
pub struct Item;
//- /right.rs crate:right
pub struct Item;
"#,
        );
        let generic = function(&db, "generic");
        let producer = function(&db, "producer");
        let producer_again = function(&db, "producer_again");
        let first = return_instance(&db, generic, producer);
        let expected = oracle(&db, &first);
        executions();
        assert_eq!(*signature_result(&db, generic, producer), expected);
        assert_eq!(executions(), 1);
        assert_eq!(*signature_result(&db, generic, producer), expected);
        assert_eq!(executions(), 0);
        assert_eq!(*signature_result(&db, generic, producer_again), expected);
        assert_eq!(executions(), 0, "a distinct tracked caller must reuse the same native key");

        // A dependency's export changes the resolved ADT without touching the main file.
        let dependency_file = files[1].file_id(&db);
        db.set_file_text(dependency_file, "pub use right::Item;");
        let second = return_instance(&db, generic, producer);
        assert_ne!(first, second, "resolved dependency type must change the instance key");
        let dependency_expected = oracle(&db, &second);
        assert_ne!(expected, dependency_expected, "the mentioned dependency crate must change");
        assert_eq!(*signature_result(&db, generic, producer), dependency_expected);
        assert_eq!(executions(), 1, "changed dependency creates one new native key");
        assert_eq!(*signature_result(&db, generic, producer), dependency_expected);
        assert_eq!(executions(), 0);
        assert_eq!(*signature_result(&db, generic, producer_again), dependency_expected);
        assert_eq!(executions(), 0, "the second caller must validate its dependency too");

        // Change the actual signature to a primitive, keeping both FunctionIds stable.
        let main_file = files[0].file_id(&db);
        db.set_file_text(main_file, "fn generic<T>() {}\nfn producer() -> u32 { 0 }");
        let third = return_instance(&db, generic, producer);
        assert_ne!(second, third);
        let signature_expected = oracle(&db, &third);
        assert_eq!(*signature_result(&db, generic, producer), signature_expected);
        assert_eq!(executions(), 1);
        assert_eq!(*signature_result(&db, generic, producer), signature_expected);
        assert_eq!(executions(), 0);

        // Body-only revision: same signature and key, no home provider execution needed.
        db.set_file_text(main_file, "fn generic<T>() {}\nfn producer() -> u32 { 1 }");
        assert_eq!(return_instance(&db, generic, producer), third);
        assert_eq!(*signature_result(&db, generic, producer), oracle(&db, &third));
        assert_eq!(executions(), 0);
    }

    #[test]
    fn native_home_real_db_closure_owner_revision() {
        let (mut db, file) = RootDatabase::with_single_file(
            "fn generic<T>() {}\nfn owner_a() { let f = || 1u32; }\nfn owner_b() {}",
        );
        let generic = function(&db, "generic");
        let first = closure_instance(&db, generic);
        executions();
        let expected = oracle(&db, &first);
        assert_eq!(*closure_result(&db, generic), expected);
        assert_eq!(executions(), 1);
        assert_eq!(*closure_result(&db, generic), expected);
        assert_eq!(executions(), 0);

        let file = file.file_id(&db);
        db.set_file_text(
            file,
            "fn generic<T>() {}\nfn owner_a() {}\nfn owner_b() { let f = || 1u32; }",
        );
        let second = closure_instance(&db, generic);
        assert_ne!(first, second, "moving the closure must change its actual owner identity");
        let changed = oracle(&db, &second);
        assert_eq!(changed, expected, "different owners in this fixture mention the same crate");
        assert_eq!(*closure_result(&db, generic), changed);
        assert_eq!(executions(), 1, "equal output must not hide a missed owner recomputation");
        assert_eq!(*closure_result(&db, generic), changed);
        assert_eq!(executions(), 0);
    }

    #[test]
    fn native_home_real_db_opaque_fallback() {
        let db = RootDatabase::with_files(
            "//- minicore: sized\nfn generic<T>() {}\nfn producer() -> impl Sized { 0u32 }",
        );
        let instance = return_instance(&db, function(&db, "generic"), function(&db, "producer"));
        executions();
        assert_eq!(oracle(&db, &instance), None, "opaque aliases must remain opaque");
        assert_eq!(wrapped(&db, &instance, true), None);
        assert_eq!(executions(), 1);
        assert_eq!(wrapped(&db, &instance, true), None);
        assert_eq!(executions(), 0);
    }
}
