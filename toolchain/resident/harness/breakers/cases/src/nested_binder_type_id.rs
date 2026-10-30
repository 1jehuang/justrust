//! tests/ui/unboxed-closures/type-id-higher-rank.rs: the `for<'a>` written on a function
//! pointer type was given to the first function pointer among its parameters, so the two
//! types here were one.

use std::any::TypeId;

pub fn run() -> (bool, bool) {
    let outer = TypeId::of::<for<'a> fn(fn(&'a isize) -> &'a isize)>();
    let inner = TypeId::of::<fn(for<'a> fn(&'a isize) -> &'a isize)>();
    let renamed = TypeId::of::<fn(for<'b> fn(&'b isize) -> &'b isize)>();
    (outer != inner, inner == renamed)
}
