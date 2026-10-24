//! Invalid source never gains authority by crossing a macro/source boundary.
#[path = "support/source_macro_binding_oracle.rs"]
mod oracle;

#[test]
fn forbid_unsafe_code_inherits_into_files_but_not_dependency_crates() {
    oracle::Fixture::new(
        "#![forbid(unsafe_code)] mod child; fn main(){assert_eq!(child::value(),42);}",
        &[("child.rs", "pub fn value()->i32{42}")],
        Some("pub unsafe fn unused(){}"),
    )
    .assert_positive();
    oracle::Fixture::new(
        "#![forbid(unsafe_code)] mod child; fn main(){}",
        &[("child.rs", "pub unsafe fn unused(){}")],
        None,
    )
    .assert_negative();
    oracle::Fixture::new(
        "fn main(){}",
        &[],
        Some("#![forbid(unsafe_code)] pub unsafe fn unused(){}"),
    )
    .assert_negative();
}

#[test]
fn shadowing_preserves_order_scope_selected_rule_and_unused_errors() {
    for source in [
        "fn f()->i32 {value!()} macro_rules! value {()=>{1i32};} fn main(){}",
        "mod child {macro_rules! value {()=>{1i32};}} fn unused()->i32 {value!()} fn main(){}",
        "macro_rules! value {($x:tt)=>{1i32};} macro_rules! value {()=>{2i32};} fn unused()->i32 {value!(token)} fn main(){}",
        "macro_rules! panic {()=>{missing_name};} fn unused()->i32 {panic!()} fn main(){}",
        "#[cfg(any())] macro_rules! value {()=>{1i32};} fn unused()->i32 {value!()} fn main(){}",
        "macro_rules! value {()=>{true};} fn unused()->i32 {value!()} fn main(){}",
        "macro_rules! panic {()=>{23i32};} fn unused()->i32 {panic!(\"not a builtin escape\")} fn main(){}",
        "macro_rules! println {()=>{31i32};} fn unused(){println!(\"not a builtin escape\")} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn dependency_cannot_capture_a_user_macro_even_in_an_unused_body() {
    oracle::Fixture::new(
        "macro_rules! private_value {()=>{67i32};} fn main(){assert_eq!(private_value!(),67i32);}",
        &[],
        Some("pub fn unused()->i32 {private_value!()}"),
    )
    .assert_negative();
}
