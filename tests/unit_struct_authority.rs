//! Public source-schema integrity controls, not private native-layout claims.
//! All name/module/field handles come from actual source parsing.
use justrust_engine::compiler::frontend::{self, PlanCtx, Program, StructShape};
use justrust_engine::compiler::ir::PlanBudget;
use std::sync::Arc;

const SOURCE: &str = r#"
struct Token;
struct Other;
struct Record { field:i32 }
fn main() { let _=Token; let _=Other{}; }
"#;

fn fixture() -> Program {
    let program = frontend::parse(SOURCE).expect("real unit and record struct source");
    assert_eq!(program.structs.len(), 3);
    let unit = structure(&program, "Token");
    let record = structure(&program, "Record");
    assert_eq!(program.structs[unit].shape, StructShape::Unit);
    assert!(program.structs[unit].fields.is_empty());
    assert_eq!(program.structs[record].shape, StructShape::Record);
    assert_eq!(program.structs[record].fields.len(), 1);
    assert_eq!(program.structs[unit].module, program.structs[record].module);
    program
}

fn structure(program: &Program, spelling: &str) -> usize {
    program
        .structs
        .iter()
        .position(|declaration| {
            declaration.name.len() == 1
                && program
                    .names
                    .spelling(declaration.name.segments()[0])
                    .unwrap()
                    == spelling
        })
        .unwrap_or_else(|| panic!("missing parsed struct {spelling}"))
}

fn reject_shape_before_authority(program: Program) {
    let budget = PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("contradictory source struct schema acquired planning authority"),
    };
    // A caller-mutated public AST is a contract violation. This differs from
    // valid but unadmitted empty-record source, whose parser policy is Unsupported.
    let diagnostic = error.to_ascii_lowercase();
    assert!(diagnostic.contains("contract"), "wrong rejection: {error}");
    assert!(
        !diagnostic.contains("unsupported"),
        "wrong boundary: {error}"
    );
    let usage = budget.usage_snapshot();
    assert_eq!(
        usage.applications, 0,
        "invalid schema published applications"
    );
    assert_eq!(usage.obligations, 0, "invalid schema published obligations");
    assert!(!usage.poisoned, "shape mismatch is not resource exhaustion");
}

#[test]
fn exact_unit_and_record_schema_clone_retains_owners_and_plans() {
    let original = fixture();
    let cloned = original.clone();
    assert!(Arc::ptr_eq(&original.names, &cloned.names));
    assert!(Arc::ptr_eq(&original.modules, &cloned.modules));
    assert!(Arc::ptr_eq(&original.symbols, &cloned.symbols));
    drop(original);
    PlanCtx::from_shared_program(Arc::new(cloned), &PlanBudget::default()).unwrap();
}

#[test]
fn unit_shape_cannot_authorize_same_owner_record_fields() {
    let mut program = fixture();
    let unit = structure(&program, "Token");
    let record = structure(&program, "Record");
    program.structs[unit].fields = program.structs[record].fields.clone();
    assert_eq!(program.structs[unit].shape, StructShape::Unit);
    reject_shape_before_authority(program);
}

#[test]
fn nonempty_record_cannot_gain_unit_constructor_authority_by_shape_flip() {
    let mut program = fixture();
    let record = structure(&program, "Record");
    program.structs[record].shape = StructShape::Unit;
    assert_eq!(program.structs[record].fields.len(), 1);
    reject_shape_before_authority(program);
}

#[test]
fn empty_fields_do_not_implicitly_certify_a_unit_source_shape() {
    let mut program = fixture();
    let unit = structure(&program, "Token");
    program.structs[unit].shape = StructShape::Record;
    assert!(program.structs[unit].fields.is_empty());
    reject_shape_before_authority(program);
}

#[test]
fn removing_record_fields_cannot_silently_promote_it_to_unit_schema() {
    let mut program = fixture();
    let record = structure(&program, "Record");
    program.structs[record].fields.clear();
    assert_eq!(program.structs[record].shape, StructShape::Record);
    reject_shape_before_authority(program);
}
