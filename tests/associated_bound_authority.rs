//! Real-source associated declaration bound authority. No synthetic facts or IDs.
use frontend::{PlanCtx, Program, TraitPath};
use ir::{AssociatedTypeProjection, FactsCtx, Ty};
use justrust_engine::compiler::traits::FactsErrorKind;
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
struct S;
struct Cell {value:i32}
struct Wrap<T> {value:T}
trait Source<A,B>:Sized {
    type Out:Copy+Sized+Copy;
    fn get(self)->Self::Out;
    fn marker(self)->i32 {17i32}
}
trait Other:Sized {type Out:Copy;}
impl Source<i32,bool> for S {type Out=i32;fn get(self)->Self::Out {37i32}}
impl Source<bool,i32> for S {type Out=(i32,bool);fn get(self)->Self::Out {(41i32,true)}}
impl Source<u8,bool> for S {type Out=i128;fn get(self)->Self::Out {43i128}}
impl<T:Copy> Source<u16,bool> for Wrap<T> {type Out=T;fn get(self)->Self::Out {self.value}}
impl Other for S {type Out=bool;}
fn scalar()->i32 {<S as Source<i32,bool>>::get(S)}
fn tuple()->(i32,bool) {<S as Source<bool,i32>>::get(S)}
fn wide()->i128 {<S as Source<u8,bool>>::get(S)}
fn generic<T:Source<i32,bool,Out=U>,U:Copy>(x:T)->(U,U) {
    let value:U=<T as Source<i32,bool>>::get(x);(value,value)
}
fn closed()->(i32,i32) {generic::<S,i32>(S)}
fn generic_closed()->i32 {<Wrap<i32> as Source<u16,bool>>::get(Wrap::<i32>{value:47i32})}
fn main() {}
"#;

fn plan(source: &str) -> (Program, PlanCtx) {
    let program = frontend::parse(source).unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    (program, plan)
}
fn free(program: &Program, plan: &PlanCtx, name: &str) -> ir::DeclId {
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap();
    plan.function_decl(index).unwrap()
}
fn check_callable(program: &Program, plan: &PlanCtx, declaration: ir::DeclId) {
    let facts = plan.callable_facts(declaration).unwrap();
    let checked =
        frontend::type_callable_with_plan(program, declaration, plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
    template
        .function()
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}
fn invalid(source: &str) {
    let program =
        frontend::parse(source).expect("representable source must reach semantic validation");
    let error = match PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
        Err(error) => error,
        Ok(_) => frontend::check(&program).expect_err("invalid source was accepted"),
    };
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}

#[test]
fn scalar_tuple_wide_and_explicit_generic_copy_reach_real_cfg_and_ownership() {
    let (program, plan) = plan(SOURCE);
    frontend::check(&program).unwrap();
    for name in [
        "scalar",
        "tuple",
        "wide",
        "generic",
        "closed",
        "generic_closed",
    ] {
        check_callable(&program, &plan, free(&program, &plan, name));
    }
    let bounds = &program.traits[0].associated_types[0].bounds;
    assert_eq!(
        bounds.len(),
        3,
        "retain duplicate ordered source occurrences"
    );
    assert!(
        bounds
            .iter()
            .all(|bound| matches!(bound, TraitPath::Source(_)))
    );
}

#[test]
fn unused_noncopy_and_missing_generic_copy_assignments_are_invalid() {
    invalid(
        r#"
struct S; struct Cell {value:i32}
trait Source {type Out:Copy;}
impl Source for S {type Out=Cell;}
fn main() {}
"#,
    );
    invalid(
        r#"
struct Wrap<T> {value:T}
trait Source {type Out:Copy;}
impl<T> Source for Wrap<T> {type Out=T;}
fn main() {}
"#,
    );
}

#[test]
fn associated_copy_does_not_manufacture_equality_rhs_copy() {
    invalid(
        r#"
trait Source:Sized {type Out:Copy;fn get(self)->Self::Out;}
fn dup<T:Source<Out=U>,U>(x:T)->(U,U) {
    let value:U=<T as Source>::get(x);(value,value)
}
fn main() {}
"#,
    );
}

#[test]
fn no_bound_keeps_noncopy_cell_assignment_valid() {
    let source = r#"
struct S; struct Cell {value:i32}
trait Source:Sized {type Out;fn get(self)->Self::Out;}
impl Source for S {type Out=Cell;fn get(self)->Self::Out {Cell{value:7i32}}}
fn cell()->Cell {<S as Source>::get(S)}
fn main() {}
"#;
    let (program, plan) = plan(source);
    frontend::check(&program).unwrap();
    check_callable(&program, &plan, free(&program, &plan, "cell"));
    assert!(program.traits[0].associated_types[0].bounds.is_empty());
}

#[test]
fn normalization_binds_real_member_arguments_and_world() {
    let (program, plan) = plan(SOURCE);
    let facts = plan
        .callable_facts(free(&program, &plan, "scalar"))
        .unwrap();
    let world = facts.world();
    let header = world
        .impl_headers()
        .iter()
        .find(|header| header.trait_ref.args.as_ref() == [Ty::Int(ir::IntTy::I32), Ty::Bool])
        .unwrap();
    let projection = AssociatedTypeProjection {
        trait_ref: header.trait_ref.clone(),
        item: header.associated_types[0].associated,
    };
    let proof = world
        .normalize_associated_type(&projection, &facts)
        .unwrap();
    assert_eq!(proof.ty(), &Ty::Int(ir::IntTy::I32));
    proof.validate_for(&projection, &facts).unwrap();
    let mut swapped = projection.clone();
    let mut args = swapped.trait_ref.args.to_vec();
    args.swap(0, 1);
    swapped.trait_ref.args = args.into();
    assert!(proof.validate_for(&swapped, &facts).is_err());
    // Both IDs are obtained from real, independently finalized source worlds.
    let (foreign_program, foreign_plan) = self::plan(SOURCE);
    let foreign_facts = foreign_plan
        .callable_facts(free(&foreign_program, &foreign_plan, "scalar"))
        .unwrap();
    assert!(proof.validate_for(&projection, &foreign_facts).is_err());
    let mut foreign_member = projection.clone();
    foreign_member.item = foreign_facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| header.trait_ref.args.as_ref() == [Ty::Int(ir::IntTy::I32), Ty::Bool])
        .unwrap()
        .associated_types[0]
        .associated;
    let error = world
        .normalize_associated_type(&foreign_member, &facts)
        .unwrap_err();
    assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
    let mut other_member = projection.clone();
    other_member.item = world
        .impl_headers()
        .iter()
        .find(|header| header.trait_ref.args.is_empty() && !header.associated_types.is_empty())
        .unwrap()
        .associated_types[0]
        .associated;
    assert_eq!(
        world
            .normalize_associated_type(&other_member, &facts)
            .unwrap_err()
            .kind(),
        FactsErrorKind::InvalidMetadata
    );
}

#[test]
fn owning_default_and_impl_replay_requires_exact_ordered_bound_occurrences() {
    for mutation in 0..5 {
        let (mut program, plan) = plan(SOURCE);
        let foreign = frontend::parse(SOURCE).unwrap();
        // Enumerate real source-backed callables and retain only the owning
        // trait default and implementations, never unrelated free functions.
        let callables: Vec<_> = (0..plan.callable_count())
            .map(|index| plan.callable_decl(index).unwrap())
            // Catalog recipes have no source AST body.
            .filter(|declaration| plan.callable_body(&program, *declaration).is_ok())
            .filter(|declaration| {
                let body = plan.callable_body(&program, *declaration).unwrap();
                !program
                    .functions
                    .iter()
                    .any(|free| std::ptr::eq(free, body))
            })
            .filter(|declaration| {
                let body = plan.callable_body(&program, *declaration).unwrap();
                let name = program
                    .names
                    .spelling(*body.name.segments().last().unwrap())
                    .unwrap();
                name == "marker" || name == "get"
            })
            .collect();
        assert_eq!(
            callables.len(),
            5,
            "four real impl bodies and the owning default"
        );
        for declaration in &callables {
            check_callable(&program.clone(), &plan, *declaration);
        }
        let bounds = &mut program.traits[0].associated_types[0].bounds;
        match mutation {
            0 => bounds.clear(),
            1 => {
                bounds.remove(2);
            }
            2 => bounds.swap(0, 1),
            3 => bounds[0] = foreign.traits[0].associated_types[0].bounds[0].clone(),
            _ => bounds[0] = bounds[2].clone(),
        }
        for declaration in callables {
            let error = frontend::type_callable_with_plan(
                &program,
                declaration,
                &plan,
                &ir::PlanBudget::default(),
            )
            .expect_err("changed owning associated bounds reused retained plan");
            assert!(
                !error.to_ascii_lowercase().contains("unsupported"),
                "{error}"
            );
        }
    }
}

#[test]
fn catalog_identity_not_user_trait_spelling_grants_copy() {
    let source = r#"
trait Copy {}
struct S;
trait Source:Sized {type Out:core::marker::Copy+std::marker::Sized;fn get(self)->Self::Out;}
impl Source for S {type Out=i32;fn get(self)->Self::Out {53i32}}
fn scalar()->i32 {<S as Source>::get(S)}
fn main() {}
"#;
    let (program, plan) = plan(source);
    frontend::check(&program).unwrap();
    check_callable(&program, &plan, free(&program, &plan, "scalar"));
    // Stock: E0277 (`i32: Copy` for the user trait). The user trait spelling
    // must never resolve to the catalog Copy.
    let program = frontend::parse(
        r#"
trait Copy {}
struct S;
trait Source {type Out:Copy;}
impl Source for S {type Out=i32;}
fn main() {}
"#,
    )
    .unwrap();
    let error = frontend::check(&program).expect_err("stock rejects with E0277");
    assert!(error.contains("E0277"), "{error}");
}

#[test]
fn duplicate_foreign_occurrence_is_validated_before_non_catalog_capability() {
    let source = r#"
trait Custom {}
trait Source {type Out:Copy+Copy+Copy;}
fn custom<T:Custom>() {}
fn main() {}
"#;
    let mut program = frontend::parse(source).unwrap();
    let foreign = frontend::parse(source).unwrap();
    frontend::check(&program).unwrap();
    // Start at a genuinely admitted source tree. The user-trait path comes
    // from a real generic bound in the same source namespace, not a fabricated
    // path. A later foreign duplicate must be admitted before capability checks.
    program.traits[1].associated_types[0].bounds[0] =
        program.functions[0].bounds[0].traits[0].clone();
    program.traits[1].associated_types[0].bounds[2] =
        foreign.traits[1].associated_types[0].bounds[2].clone();
    let error = match PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
        Err(error) => error,
        Ok(_) => panic!("foreign duplicate occurrence bypassed validation"),
    };
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}

#[test]
fn foreign_bound_source_handle_is_invalid_even_with_identical_spelling() {
    let mut program = frontend::parse(SOURCE).unwrap();
    let foreign = frontend::parse(SOURCE).unwrap();
    program.traits[0].associated_types[0].bounds[0] =
        foreign.traits[0].associated_types[0].bounds[0].clone();
    let error = match PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
        Err(error) => error,
        Ok(_) => panic!("foreign associated-bound occurrence was accepted"),
    };
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}
