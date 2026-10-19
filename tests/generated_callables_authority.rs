//! Mutations use real parsed handles and cannot mint container certificates.
use justrust_engine::compiler::frontend::{self, PlanCtx, Program};
use justrust_engine::compiler::ir::PlanBudget;
use std::sync::Arc;

const SOURCE: &str = r#"
struct A; struct B;
trait First<T,U>{fn value(x:T,y:U)->T;}
trait Second<T,U>{fn value(x:T,y:U)->T;}
macro_rules! owners {()=>{
    impl First<i32,u8> for A {fn value(x:i32,y:u8)->i32{x}}
    impl Second<i32,u8> for B {fn value(x:i32,y:u8)->i32{x}}
};}
owners!();
impl A {fn donor(x:i32,y:u8)->i32{x}}
mod elsewhere {fn marker(){}}
fn main(){}
"#;
fn fixture() -> Program {
    frontend::parse(SOURCE).expect("authenticated generated impl fixture")
}
fn reject(program: Program) {
    let error = match PlanCtx::from_shared_program(Arc::new(program), &PlanBudget::default()) {
        Ok(_) => panic!("forged generated owner reached planning authority"),
        Err(error) => error,
    };
    assert!(
        error.contains("container")
            || error.contains("source")
            || error.contains("foreign")
            || error.contains("scope")
            || error.contains("handle"),
        "{error}"
    );
}
#[test]
fn exact_original_generated_impls_keep_their_headers() {
    PlanCtx::from_shared_program(Arc::new(fixture()), &PlanBudget::default()).unwrap();
}
#[test]
fn same_module_methods_cannot_be_transplanted_between_impls() {
    let mut program = fixture();
    let first = program.impls[0].items[0].clone();
    program.impls[0].items[0] = program.impls[1].items[0].clone();
    program.impls[1].items[0] = first;
    reject(program);
}
#[test]
fn exact_trait_and_self_headers_cannot_be_replaced() {
    for change_trait in [false, true] {
        let mut program = fixture();
        if change_trait {
            program.impls[0].trait_path = program.impls[1].trait_path.clone();
        } else {
            program.impls[0].self_ty = program.impls[1].self_ty.clone();
        }
        reject(program);
    }
}
#[test]
fn ordered_trait_actuals_cannot_be_swapped() {
    let mut program = fixture();
    let frontend::TraitPath::Source(path) = program.impls[0].trait_path.as_mut().unwrap() else {
        panic!()
    };
    path.segments.last_mut().unwrap().args.swap(0, 1);
    reject(program);
}
#[test]
fn missing_extra_and_duplicate_products_do_not_close_the_inventory() {
    for operation in 0..3 {
        let mut program = fixture();
        match operation {
            0 => {
                program.impls[0].items.clear();
            }
            1 => {
                let extra = program.impls[0].items[0].clone();
                program.impls[1].items.push(extra);
            }
            _ => {
                let duplicate = program.impls[0].clone();
                program.impls.push(duplicate);
            }
        }
        reject(program);
    }
}
#[test]
fn same_shaped_foreign_impl_is_not_source_authority() {
    let mut program = fixture();
    let foreign = fixture();
    program.impls[0] = foreign.impls[0].clone();
    reject(program);
}

#[test]
fn ordinary_source_member_cannot_be_inserted_into_a_generated_container() {
    let mut program = fixture();
    let donor = program.impls[2].items[0].clone();
    program.impls[0].items.push(donor);
    reject(program);
}

#[test]
fn generated_container_cannot_be_assigned_to_another_real_module() {
    let mut program = fixture();
    let module = program
        .functions
        .iter()
        .find(|function| {
            program
                .names
                .spelling(*function.name.segments().last().unwrap())
                .unwrap()
                == "marker"
        })
        .unwrap()
        .module;
    program.impls[0].module = module;
    reject(program);
}
