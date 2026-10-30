//! rustc loads a dependency when something names it, and knows the impls of the crates it
//! loaded. `unnamed` is a dev-dependency nothing here names, so its `impl PartialEq<Value> for
//! f64` is not there and `parse` below is of an `f64`. ra-jit knew the impls of every crate
//! below this one and could not tell which `parse` it was: ryu's tests, whose benches bring in
//! serde_json and its `impl PartialEq<Value> for f64`.

#[test]
fn parses_a_float() {
    assert_eq!(1.5, "1.5".parse().unwrap());
}

#[test]
fn compares_floats() {
    let parsed = "2.5e3".parse().unwrap();
    assert!(2500.0 == parsed);
}
