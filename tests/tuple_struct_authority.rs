//! Public tuple source-schema integrity, not native layout or full Rust support.
//! Every name, module, and field handle is obtained from real source parsing.
use justrust_engine::compiler::frontend::{self, FieldName, PlanCtx, Program, StructShape};
use justrust_engine::compiler::ir::PlanBudget;
use std::sync::Arc;

const SOURCE: &str = r#"
struct Pair(i32, bool);
struct Record { field:i32 }
struct Token;
fn main() {}
"#;

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

fn fixture() -> Program {
    let program = frontend::parse(SOURCE).expect("real tuple, record, and unit struct source");
    assert_eq!(program.structs.len(), 3);
    let pair = structure(&program, "Pair");
    let record = structure(&program, "Record");
    let token = structure(&program, "Token");
    assert_eq!(program.structs[pair].shape, StructShape::Tuple);
    assert_eq!(program.structs[pair].fields.len(), 2);
    assert!(matches!(
        program.structs[pair].fields[0].name,
        FieldName::Tuple(0)
    ));
    assert!(matches!(
        program.structs[pair].fields[1].name,
        FieldName::Tuple(1)
    ));
    assert_eq!(program.structs[record].shape, StructShape::Record);
    assert!(matches!(
        program.structs[record].fields[0].name,
        FieldName::Named(_)
    ));
    assert_eq!(program.structs[token].shape, StructShape::Unit);
    assert!(program.structs[token].fields.is_empty());
    assert_eq!(program.structs[pair].module, program.structs[record].module);
    program
}

fn reject_before_authority(program: Program) {
    let budget = PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("malformed public tuple schema acquired planning authority"),
    };
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
    assert!(
        !usage.poisoned,
        "schema mismatch is not resource exhaustion"
    );
}

#[test]
fn exact_tuple_schema_clone_retains_owners_and_plans_unused_declarations() {
    let original = fixture();
    let cloned = original.clone();
    assert!(Arc::ptr_eq(&original.names, &cloned.names));
    assert!(Arc::ptr_eq(&original.modules, &cloned.modules));
    assert!(Arc::ptr_eq(&original.symbols, &cloned.symbols));
    drop(original);
    PlanCtx::from_shared_program(Arc::new(cloned), &PlanBudget::default()).unwrap();
}

#[test]
fn tuple_shape_rejects_same_owner_named_field_occurrence() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    let record = structure(&program, "Record");
    program.structs[pair].fields[0].name = program.structs[record].fields[0].name;
    reject_before_authority(program);
}

#[test]
fn record_shape_rejects_positional_field_without_tuple_authority() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    let record = structure(&program, "Record");
    program.structs[record].fields[0].name = program.structs[pair].fields[0].name;
    reject_before_authority(program);
}

#[test]
fn tuple_shape_rejects_duplicate_positional_indices() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields[1].name = FieldName::Tuple(0);
    reject_before_authority(program);
}

#[test]
fn tuple_shape_rejects_gap_in_positional_indices() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields[1].name = FieldName::Tuple(2);
    reject_before_authority(program);
}

#[test]
fn tuple_shape_rejects_reordered_positional_fields() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields.swap(0, 1);
    reject_before_authority(program);
}

#[test]
fn tuple_shape_rejects_maximum_index_without_overflow_or_resource_failure() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields[1].name = FieldName::Tuple(usize::MAX);
    reject_before_authority(program);
}

#[test]
fn removing_all_tuple_fields_is_contract_not_zero_tuple_admission() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields.clear();
    reject_before_authority(program);
}

#[test]
fn nonempty_tuple_cannot_gain_unit_constructor_authority_by_shape_flip() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].shape = StructShape::Unit;
    reject_before_authority(program);
}

#[test]
fn unit_declaration_cannot_gain_tuple_schema_by_shape_flip() {
    let mut program = fixture();
    let token = structure(&program, "Token");
    program.structs[token].shape = StructShape::Tuple;
    reject_before_authority(program);
}

#[test]
fn tuple_fields_cannot_gain_record_schema_by_shape_flip() {
    let mut program = fixture();
    let pair = structure(&program, "Pair");
    program.structs[pair].shape = StructShape::Record;
    reject_before_authority(program);
}
