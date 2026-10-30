use test_fixture::WithFixture;

use crate::{db::HirDatabase, setup_tracing, test_db::TestDB};

fn lower_mir(#[rust_analyzer::rust_fixture] ra_fixture: &str) {
    let _tracing = setup_tracing();
    let (db, file_ids) = TestDB::with_many_files(ra_fixture);
    crate::attach_db(db.as_dyn(), || {
        let file_id = *file_ids.last().unwrap();
        let module_id = db.module_for_file(file_id.file_id(&db));
        let def_map = module_id.def_map(&db);
        let scope = &def_map.module(&db, module_id).scope(&db);
        let funcs = scope.declarations().filter_map(|x| match x {
            hir_def::ModuleDefId::FunctionId(it) => Some(it),
            _ => None,
        });
        for func in funcs {
            _ = db.mir_body(func.into());
        }
    })
}

/// Lowers every function of the fixture, each of which has to lower.
fn lower_mir_all(#[rust_analyzer::rust_fixture] ra_fixture: &str) {
    let _tracing = setup_tracing();
    let (db, file_ids) = TestDB::with_many_files(ra_fixture);
    crate::attach_db(db.as_dyn(), || {
        let file_id = *file_ids.last().unwrap();
        let module_id = db.module_for_file(file_id.file_id(&db));
        let def_map = module_id.def_map(&db);
        let scope = &def_map.module(&db, module_id).scope(&db);
        let funcs = scope.declarations().filter_map(|x| match x {
            hir_def::ModuleDefId::FunctionId(it) => Some(it),
            _ => None,
        });
        for func in funcs {
            if let Err(e) = db.mir_body(func.into()) {
                panic!("{func:?} does not lower: {e:?}");
            }
        }
    })
}

#[test]
fn builtin_macro_last_in_a_block() {
    // A builtin's expansion is rust-analyzer's own (`builtin #asm (..)`), not the statements
    // rustc parses: a call of one in parentheses at a block's end is not refused (#154).
    lower_mir_all(
        r#"
//- minicore: asm
fn f() {
    unsafe { core::arch::asm!("nop") }
}
"#,
    );
}

#[test]
fn dyn_projection_with_auto_traits_regression_next_solver() {
    lower_mir(
        r#"
//- minicore: sized, send
pub trait Deserializer {}

pub trait Strictest {
    type Object: ?Sized;
}

impl Strictest for dyn CustomValue {
    type Object = dyn CustomValue + Send;
}

pub trait CustomValue: Send {}

impl CustomValue for () {}

struct Box<T: ?Sized>;

type DeserializeFn<T> = fn(&mut dyn Deserializer) -> Box<T>;

fn foo() {
    (|deserializer| Box::new(())) as DeserializeFn<<dyn CustomValue as Strictest>::Object>;
}
    "#,
    );
}

#[test]
fn ref_pattern_on_unresolved_alias() {
    lower_mir(
        r#"
//- minicore: sized
trait Tr {
    type A;
}

fn f<T: Tr>(x: T::A) {
    let &() = x;
}
"#,
    );
}

#[test]
fn nested_closure_moves_local_of_enclosing_closure() {
    let (db, file_id) = TestDB::with_single_file(
        r#"
//- minicore: copy, fn
struct S;
fn main() {
    let outer = S;
    || {
        let local = outer;
        || { let moved = local; };
    };
}
"#,
    );
    crate::attach_db(db.as_dyn(), || {
        let module_id = db.module_for_file(file_id.file_id(&db));
        let scope = &module_id.def_map(&db).module(&db, module_id).scope(&db);
        for decl in scope.declarations() {
            if let hir_def::ModuleDefId::FunctionId(func) = decl {
                let mut closures = db.mir_body(func.into()).unwrap().closures.clone();
                while let Some(closure) = closures.pop() {
                    closures.extend(&db.mir_body_for_closure(closure).unwrap().closures);
                }
            }
        }
    })
}
