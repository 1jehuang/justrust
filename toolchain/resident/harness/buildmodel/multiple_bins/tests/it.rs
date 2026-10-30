#[test]
fn from_outside() {
    assert_eq!(app::shared("it"), "shared by it, test build: false");
}

#[test]
fn second_one() {
    assert!(app::shared("x").starts_with("shared"));
}
