#[test]
fn from_outside() {
    assert_eq!(clippy_fixture::maps(Some(1), Ok(2)) > 0, true);
}
