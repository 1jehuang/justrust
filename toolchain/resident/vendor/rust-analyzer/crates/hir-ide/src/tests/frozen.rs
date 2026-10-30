//! Frozen impls and lang items (`hir_ty::method_resolution::frozen`, `hir_def`'s
//! `nameres::frozen`): read back, they equal what finding them gives, impls in bodies and in
//! the blocks of macro expansions too.

use std::sync::Mutex;

use base_db::{Crate, SourceDatabase, all_crates};
use hir_def::nameres::frozen::{self as frozen_def, Frozen, Mode};
use rustc_hash::FxHashMap;
use test_fixture::WithFixture;

use crate::{method_resolution::frozen as frozen_impls, test_db::TestDB};

const FIXTURE: &str = r#"
//- minicore: derive, clone, copy, sized, fn
//- /main.rs crate:main deps:lib
use lib::{Show, Wrap};
#[derive(Clone, Copy)]
struct Local(u8);
impl Show for Local {}
impl Local { fn new() -> Self { Local(0) } }
fn main() {
    struct Inner;
    impl Show for Inner {}
    impl Inner { fn f() {} }
    {
        struct Deeper;
        impl Show for Deeper {}
    }
    let _ = Wrap(Local::new()).get();
}
macro_rules! in_const {
    ($t:ident) => {
        const _: () = {
            struct $t;
            impl Show for $t {}
            fn g() { struct H; impl Show for H {} }
        };
    };
}
in_const!(Made);
enum Either { A = { struct InVariant; impl Show for InVariant {} 0 }, B }

//- /lib.rs crate:lib
pub trait Show {}
pub trait Any {}
pub struct Wrap<T>(pub T);
impl Show for u8 {}
impl Show for i128 {}
impl Show for f64 {}
impl Show for bool {}
impl Show for char {}
impl Show for &str {}
impl Show for &mut u8 {}
impl<T> Show for [T] {}
impl<T, const N: usize> Show for [T; N] {}
impl<T> Show for *const T {}
impl<T> Show for *mut T {}
impl Show for (u8, u16) {}
impl Show for fn(u8) -> u8 {}
impl Show for dyn Any {}
impl<T: Show> Show for Wrap<T> {}
impl<T> Any for T {}
impl<T> Wrap<T> { pub fn get(self) -> T { self.0 } }
"#;

/// The differences are recorded for the whole process: the tests that read them take turns.
static TURN: Mutex<()> = Mutex::new(());

fn crates(db: &TestDB) -> Vec<Crate> {
    all_crates(db).iter().copied().collect()
}

fn name(db: &TestDB, krate: Crate) -> String {
    krate.extra_data(db).display_name.as_ref().map(|it| it.to_string()).unwrap_or_default()
}

/// The frozen def maps, lang items and impls of each crate of a fresh database, by name.
struct Bytes {
    def_maps: Vec<(String, Vec<u8>)>,
    lang_items: Vec<(String, Vec<u8>)>,
    impls: Vec<(String, Vec<u8>)>,
}

fn frozen() -> Bytes {
    let db = TestDB::with_files(FIXTURE);
    crate::attach_db(&db, || {
        let all = crates(&db);
        let each = |encode: &dyn Fn(Crate) -> Result<Vec<u8>, String>| {
            all.iter().map(|&it| (name(&db, it), encode(it).unwrap())).collect()
        };
        Bytes {
            def_maps: each(&|it| frozen_def::encode(&db, it)),
            lang_items: each(&|it| frozen_def::encode_lang_items(&db, it)),
            impls: each(&|it| frozen_impls::encode(&db, it)),
        }
    })
}

fn by_crate(db: &TestDB, mode: Mode, bytes: &[(String, Vec<u8>)]) -> FxHashMap<Crate, Frozen> {
    let crates = crates(db);
    bytes
        .iter()
        .map(|(name, bytes)| {
            let krate = *crates.iter().find(|&&it| self::name(db, it) == *name).unwrap();
            (krate, Frozen { bytes: bytes.as_slice().into(), mode })
        })
        .collect()
}

/// A fresh database with the impls and lang items of [`Bytes`] in `mode`, and the def maps
/// too when `def_maps`.
fn with(mode: Mode, bytes: &Bytes, def_maps: bool) -> TestDB {
    let mut db = TestDB::with_files(FIXTURE);
    if def_maps {
        let maps = by_crate(&db, mode, &bytes.def_maps);
        frozen_def::set(&mut db, maps);
    }
    let lang_items = by_crate(&db, mode, &bytes.lang_items);
    frozen_def::set_lang_items(&mut db, lang_items);
    let impls = by_crate(&db, mode, &bytes.impls);
    frozen_impls::set(&mut db, impls);
    db
}

/// The impls and lang items of every crate of `db`, written as they are now.
fn written(db: &TestDB) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    crate::attach_db(db, || {
        crates(db)
            .into_iter()
            .map(|it| {
                let impls = frozen_impls::encode(db, it).unwrap();
                (name(db, it), impls, frozen_def::encode_lang_items(db, it).unwrap())
            })
            .collect()
    })
}

fn expected(bytes: &Bytes) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    bytes
        .impls
        .iter()
        .zip(&bytes.lang_items)
        .map(|((name, impls), (_, lang_items))| (name.clone(), impls.clone(), lang_items.clone()))
        .collect()
}

#[test]
fn the_same_bytes_in_every_database() {
    let (one, other) = (frozen(), frozen());
    assert_eq!(one.impls, other.impls);
    assert_eq!(one.lang_items, other.lang_items);
}

#[test]
fn the_fixture_has_impls_in_blocks() {
    let db = TestDB::with_files(FIXTURE);
    let all = crates(&db);
    let named = |it: &str| *all.iter().find(|&&krate| name(&db, krate) == it).unwrap();
    let lib = hir_def::nameres::crate_def_map(&db, named("lib")).root_module_id();
    let scope = lib.def_map(&db).module(&db, lib).scope(&db);
    let Some(hir_def::ModuleDefId::TraitId(show)) =
        scope.get(&hir_expand::name::Name::new_root("Show")).take_types()
    else {
        panic!("no trait `Show`")
    };
    let mut in_blocks = 0;
    crate::method_resolution::TraitImpls::for_crate(&db, named("main")).for_trait(show, |it| {
        if let either::Either::Left(impls) = it {
            in_blocks +=
                impls.iter().filter(|it| it.loc(&db).container.block(&db).is_some()).count();
        }
    });
    // `Inner`, `Deeper`, `Made` and `H`, whose block is in a function in the block of a const.
    assert_eq!(in_blocks, 4);
}

#[test]
fn read_back_writes_the_same_bytes() {
    let _turn = TURN.lock().unwrap_or_else(|it| it.into_inner());
    let bytes = frozen();
    for def_maps in [false, true] {
        let db = with(Mode::Verify, &bytes, def_maps);
        assert_eq!(written(&db), expected(&bytes), "def maps frozen: {def_maps}");
        crate::attach_db(&db, || {
            for krate in crates(&db) {
                frozen_impls::check(&db, krate);
                frozen_def::check_lang_items(&db, krate);
            }
        });
        assert_eq!(frozen_def::differences(), Vec::<String>::new());
    }
}

#[test]
fn served_resolves_the_same() {
    let bytes = frozen();
    let served = with(Mode::Serve, &bytes, true);
    for krate in crates(&served) {
        assert!(frozen_impls::served(&served, krate), "{}", name(&served, krate));
    }
    assert_eq!(written(&served), expected(&bytes));
}

/// A library whose items procedural macros derive and rewrite, beside the crate of those macros.
const PROC_MACROS: &str = r#"
//- proc_macros: identity, derive_identity
//- minicore: derive
//- /main.rs crate:main deps:lib
use lib::{made, Shown};
#[proc_macros::identity]
fn main() { made(); let _ = Shown; }
//- /lib.rs crate:lib
#[derive(proc_macros::DeriveIdentity)]
pub struct Shown;
#[proc_macros::identity]
pub fn made() {}
pub use proc_macros::DeriveIdentity;
"#;

#[test]
fn def_maps_with_procedural_macros_read_back_as_resolved() {
    let _turn = TURN.lock().unwrap_or_else(|it| it.into_inner());
    let db = TestDB::with_files(PROC_MACROS);
    let bytes: Vec<(String, Vec<u8>)> = crate::attach_db(&db, || {
        let encoded =
            |it| frozen_def::encode(&db, it).map_err(|why| format!("{}: {why}", name(&db, it)));
        crates(&db)
            .into_iter()
            .map(|it| Ok((name(&db, it), encoded(it)?)))
            .collect::<Result<_, String>>()
    })
    .unwrap();
    assert!(bytes.iter().any(|(name, _)| name == "proc_macros"), "{bytes:?}");
    for mode in [Mode::Check, Mode::Verify] {
        let mut db = TestDB::with_files(PROC_MACROS);
        let maps = by_crate(&db, mode, &bytes);
        frozen_def::set(&mut db, maps);
        crate::attach_db(&db, || {
            for krate in crates(&db) {
                _ = hir_def::nameres::crate_def_map(&db, krate);
            }
        });
        assert_eq!(frozen_def::differences(), Vec::<String>::new(), "{mode:?}");
    }
}

#[test]
fn checking_finds_a_difference() {
    let _turn = TURN.lock().unwrap_or_else(|it| it.into_inner());
    let mut bytes = frozen();
    let check = |bytes: &Bytes| {
        let db = with(Mode::Check, bytes, false);
        crate::attach_db(&db, || {
            for krate in crates(&db) {
                assert!(!frozen_impls::served(&db, krate));
                frozen_impls::check(&db, krate);
                frozen_def::check_lang_items(&db, krate);
            }
        });
        frozen_def::differences()
    };
    assert_eq!(check(&bytes), Vec::<String>::new());
    bytes.impls[0].1.push(0);
    bytes.lang_items[1].1.push(0);
    let found = check(&bytes);
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found.iter().any(|it| it.starts_with("the frozen impls of")), "{found:#?}");
    assert!(found.iter().any(|it| it.starts_with("the frozen lang items of")), "{found:#?}");
}
