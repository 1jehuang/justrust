//! Public AST mutation controls for the bounded original-source item macro client.
//! Every handle comes from actual parsing. These tests grant no macro/body proof.
use justrust_engine::compiler::frontend::{self, AstPath, Expr, PlanCtx, Program};
use justrust_engine::compiler::ir::PlanBudget;
use justrust_engine::compiler::semantic_name::Namespace;
use std::sync::Arc;

const SOURCE: &str = r#"
macro_rules! emit {
    ($name:ident, $param:ident) => {
        fn $name($param:i32)->i32 {
            let local:i32=$param;
            local
        }
    };
}
emit!(first, input);
emit!(second, input);
fn donor()->i32 {
    macro_rules! donor_marker { () => { 0 }; }
    let first:i32=3;
    first
}
fn main() {}
"#;

fn fixture() -> Program {
    let program = frontend::parse(SOURCE).expect("real original-source item macro fixture");
    assert_eq!(program.functions.len(), 4);
    let first = function(&program, "first");
    let second = function(&program, "second");
    assert_eq!(
        program.functions[first].module,
        program.functions[second].module
    );
    let first_scope = program.functions[first]
        .lexical_scope
        .expect("first emitted function needs a real function scope");
    let second_scope = program.functions[second]
        .lexical_scope
        .expect("second invocation needs a separate function scope");
    assert_ne!(first_scope, second_scope);
    assert!(program.scopes.is_some());
    // Both emitted functions are unused by the executable root. Authority
    // validation must not wait for either callable to be demanded or typed.
    let main = &program.functions[function(&program, "main")];
    assert!(main.body.statements.is_empty());
    assert!(main.body.tail.is_none());
    program
}

fn function(program: &Program, spelling: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == spelling
        })
        .unwrap_or_else(|| panic!("missing real parsed function {spelling}"))
}

fn reject_before_authority(program: Program, require_contract: bool) {
    let budget = PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("transplanted item product acquired planning authority"),
    };
    // The existing public planning API renders diagnostics as String. Pin the
    // contract category rather than accepting unrelated syntax/type rejection.
    let diagnostic = error.to_ascii_lowercase();
    if require_contract {
        assert!(diagnostic.contains("contract"), "wrong rejection: {error}");
    } else {
        assert!(
            diagnostic.contains("contract") || diagnostic.contains("foreign"),
            "wrong owner rejection: {error}"
        );
    }
    assert!(
        !diagnostic.contains("unsupported"),
        "wrong boundary: {error}"
    );
    let usage = budget.usage_snapshot();
    assert_eq!(usage.applications, 0, "rejected AST published applications");
    assert_eq!(usage.obligations, 0, "rejected AST published obligations");
    assert!(!usage.poisoned, "owner mismatch is not resource exhaustion");
}

#[test]
fn exact_parsed_clone_retains_item_product_owners_and_plans() {
    let original = fixture();
    let cloned = original.clone();
    assert!(Arc::ptr_eq(&original.names, &cloned.names));
    assert!(Arc::ptr_eq(&original.modules, &cloned.modules));
    assert!(Arc::ptr_eq(&original.symbols, &cloned.symbols));
    assert!(Arc::ptr_eq(
        original.scopes.as_ref().unwrap(),
        cloned.scopes.as_ref().unwrap()
    ));
    drop(original);
    PlanCtx::from_shared_program(Arc::new(cloned), &PlanBudget::default()).unwrap();
}

#[test]
fn distinct_item_invocations_cannot_exchange_function_scope_authority() {
    let mut program = fixture();
    let first = function(&program, "first");
    let second = function(&program, "second");
    let first_scope = program.functions[first].lexical_scope;
    program.functions[first].lexical_scope = program.functions[second].lexical_scope;
    program.functions[second].lexical_scope = first_scope;
    reject_before_authority(program, true);
}

#[test]
fn same_owner_same_spelling_source_occurrence_cannot_replace_product_name() {
    let mut program = fixture();
    let first = function(&program, "first");
    let donor = function(&program, "donor");
    let Some(tail) = program.functions[donor].body.tail.as_ref() else {
        panic!("donor needs its real parsed local reference")
    };
    let Expr::Var(reference) = tail.as_ref() else {
        panic!("donor tail must be an actual local-reference path")
    };
    assert_eq!(reference.len(), 1);
    let replacement = reference.segments()[0];
    let original = program.functions[first].name.segments()[0];
    assert_eq!(program.names.spelling(replacement).unwrap(), "first");
    assert_eq!(program.names.spelling(original).unwrap(), "first");
    assert_ne!(
        program.names.span(replacement).unwrap(),
        program.names.span(original).unwrap()
    );
    let new_path = AstPath::new(
        program.names.as_ref(),
        program.functions[first].name.anchor(),
        &[replacement],
    )
    .unwrap();
    assert_eq!(
        new_path.key(Namespace::Value),
        program.functions[first].name.key(Namespace::Value),
        "the mutation must preserve semantic path equality, not just spelling"
    );
    program.functions[first].name = new_path;
    reject_before_authority(program, true);
}

#[test]
fn same_owner_different_product_name_path_cannot_rebind_function() {
    let mut program = fixture();
    let first = function(&program, "first");
    let second = function(&program, "second");
    program.functions[first].name = program.functions[second].name.clone();
    reject_before_authority(program, true);
}

#[test]
fn unused_item_body_from_another_invocation_cannot_acquire_function_association() {
    let mut program = fixture();
    let first = function(&program, "first");
    let second = function(&program, "second");
    program.functions[first].body = program.functions[second].body.clone();
    reject_before_authority(program, true);
}

#[test]
fn swapping_both_unused_bodies_does_not_repair_item_product_authority() {
    let mut program = fixture();
    let first = function(&program, "first");
    let second = function(&program, "second");
    let first_body = program.functions[first].body.clone();
    program.functions[first].body = program.functions[second].body.clone();
    program.functions[second].body = first_body;
    reject_before_authority(program, true);
}

#[test]
fn removing_unused_generated_function_leaves_rejected_orphan_product() {
    let mut program = fixture();
    let first = function(&program, "first");
    program.functions.remove(first);
    // Keep main, every owner, and the immutable sealed product inventory. A
    // validator that checks only surviving function spans would miss this.
    function(&program, "main");
    assert!(program.scopes.is_some());
    reject_before_authority(program, true);
}

#[test]
fn removing_all_generated_functions_does_not_erase_sealed_product_coverage() {
    let mut program = fixture();
    let first_scope = program.functions[function(&program, "first")].lexical_scope;
    let second_scope = program.functions[function(&program, "second")].lexical_scope;
    program.functions.retain(|function| {
        function.lexical_scope != first_scope && function.lexical_scope != second_scope
    });
    assert_eq!(program.functions.len(), 2);
    function(&program, "main");
    assert!(program.scopes.is_some());
    reject_before_authority(program, true);
}

#[test]
fn ordinary_source_scope_name_and_empty_body_cannot_hide_orphan_item_product() {
    let mut program = fixture();
    let first = function(&program, "first");
    let donor = function(&program, "donor");
    let main = function(&program, "main");
    let generated_scope = program.functions[first].lexical_scope.unwrap();
    let ordinary_scope = program.functions[donor]
        .lexical_scope
        .expect("real original local macro creates donor function scope");
    assert_ne!(generated_scope, ordinary_scope);
    // Remove every generated occurrence from this function shell. The scope,
    // declaration path, result and empty body are all real parsed source data.
    // Only complete sealed-product coverage can still see the missing product.
    let mut replacement = program.functions[donor].clone();
    replacement.body = program.functions[main].body.clone();
    replacement.result = program.functions[main].result.clone();
    assert!(replacement.params.is_empty());
    assert!(replacement.body.statements.is_empty());
    assert!(replacement.body.tail.is_none());
    program.functions[first] = replacement;
    // Avoid relying on an incidental duplicate-name rejection. Keep exactly one
    // ordinary donor declaration and the genuine main alongside the other item.
    program.functions.remove(donor);
    assert_eq!(program.functions.len(), 3);
    function(&program, "main");
    reject_before_authority(program, true);
}

#[test]
fn foreign_name_store_cannot_rebind_existing_item_products() {
    let mut program = fixture();
    let foreign = fixture();
    assert!(!Arc::ptr_eq(&program.names, &foreign.names));
    program.names = foreign.names.clone();
    reject_before_authority(program, false);
}

#[test]
fn foreign_module_arena_cannot_rebind_existing_item_products() {
    let mut program = fixture();
    let foreign = fixture();
    assert!(!Arc::ptr_eq(&program.modules, &foreign.modules));
    program.modules = foreign.modules.clone();
    reject_before_authority(program, false);
}

#[test]
fn foreign_scope_arena_cannot_rebind_existing_item_products() {
    let mut program = fixture();
    let foreign = fixture();
    assert!(!Arc::ptr_eq(
        program.scopes.as_ref().unwrap(),
        foreign.scopes.as_ref().unwrap()
    ));
    program.scopes = foreign.scopes.clone();
    reject_before_authority(program, false);
}

#[test]
fn generated_generic_call_canonical_target_rejects_same_key_donor_occurrence() {
    use justrust_engine::compiler::semantic_name::LookupClass;

    let source = r#"
fn helper<T>(value:T)->T { value }
macro_rules! emit {
    ($name:ident, $param:ident) => {
        fn $name($param:i32)->i32 { helper::<i32>($param) }
    };
}
emit!(generated, input);
fn donor()->i32 { let helper:i32=3; helper }
fn main() {}
"#;
    let mut program = frontend::parse(source).expect("real generated generic-call fixture");
    let generated = function(&program, "generated");
    assert!(program.functions[generated].type_params.is_empty());
    assert!(program.functions[generated].lexical_scope.is_some());

    // The real parsed canonical certificate must admit the unchanged clone
    // before the mutation. Never construct or repair that private certificate.
    let cloned = program.clone();
    assert!(Arc::ptr_eq(&program.names, &cloned.names));
    assert!(Arc::ptr_eq(&program.modules, &cloned.modules));
    assert!(Arc::ptr_eq(
        program.scopes.as_ref().unwrap(),
        cloned.scopes.as_ref().unwrap()
    ));
    PlanCtx::from_shared_program(Arc::new(cloned), &PlanBudget::default()).unwrap();

    let donor = function(&program, "donor");
    let Some(Expr::Var(reference)) = program.functions[donor].body.tail.as_deref() else {
        panic!("donor tail must retain its real local helper occurrence")
    };
    assert_eq!(reference.len(), 1);
    let replacement = reference.segments()[0];
    let Some(Expr::Apply { path, value }) = program.functions[generated].body.tail.as_deref()
    else {
        panic!("generated tail must retain its real explicit generic application")
    };
    assert!(matches!(value.as_ref(), Expr::Call { args, .. } if args.len() == 1));
    let terminal = path
        .segments
        .last()
        .expect("canonical helper target segment");
    assert_eq!(terminal.args.len(), 1);
    let original = terminal.name;
    assert_eq!(program.names.spelling(original).unwrap(), "helper");
    assert_eq!(program.names.spelling(replacement).unwrap(), "helper");
    assert_eq!(
        program
            .names
            .key(original, LookupClass::Item(Namespace::Value))
            .unwrap(),
        program
            .names
            .key(replacement, LookupClass::Item(Namespace::Value))
            .unwrap(),
        "the replacement must preserve the exact semantic lookup key"
    );
    assert_ne!(
        program.names.span(original).unwrap(),
        program.names.span(replacement).unwrap(),
        "the replacement must nevertheless be a different source occurrence"
    );

    let Some(Expr::Apply { path, .. }) = program.functions[generated].body.tail.as_deref_mut()
    else {
        unreachable!("application shape checked above")
    };
    path.segments.last_mut().unwrap().name = replacement;
    reject_before_authority(program, true);
}
