#[test]
fn an_expression_is_a_group() {
    assert_eq!(fragment::handed_on(), ("name", 3, "text"));
}
