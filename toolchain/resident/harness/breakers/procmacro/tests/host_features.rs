//! wasmtime's macros build wasmparser with its hash maps and wasmtime builds it without, so
//! cargo compiles it twice. ra-jit gave the crate the features of both, and read wasmtime's
//! values with the layout the macros' build has.

#[test]
fn records() {
    assert_eq!(procmacro::records(), (vec![1, 5], 24));
}
