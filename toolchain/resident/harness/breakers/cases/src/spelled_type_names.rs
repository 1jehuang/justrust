//! library/coretests any.rs, `distinct_type_names` and `dyn_type_name`, std's
//! type-name-unsized.rs and tests/ui/type/type-name-basic.rs: `type_name` of a trait object
//! with associated types, with `Fn` sugar or with auto traits alone, of a function pointer,
//! of a constructor and of an `async` block. ra-jit had no spelling for these and left their
//! callers to rustc.

use std::any::{type_name, type_name_of_val};
use std::borrow::Cow;

trait Keyed {
    type Key;
}

trait Borrowing<'a, T> {}

struct Pair(#[allow(dead_code)] f32, #[allow(dead_code)] f32);

struct Wrapped<T>(#[allow(dead_code)] T);

enum Either<T> {
    #[allow(dead_code)]
    Left(T),
}

fn trait_objects() -> Vec<&'static str> {
    vec![
        type_name::<dyn Fn(i32, i32) -> i32>(),
        type_name::<dyn FnOnce()>(),
        type_name::<dyn FnMut(u8) + Send>(),
        type_name::<dyn for<'a> Fn(&'a u8) -> &'a u8>(),
        type_name::<dyn Keyed<Key = i32> + Sync + Send>(),
        type_name::<dyn Send>(),
        type_name::<dyn Borrowing<'static, Cow<'static, ()>>>(),
        type_name::<dyn Iterator<Item = u8>>(),
    ]
}

fn function_pointers() -> Vec<&'static str> {
    vec![
        type_name::<fn(i32) -> i32>(),
        type_name::<fn(&'static u32)>(),
        type_name::<for<'a> fn(&'a mut u32, &'static u8)>(),
        type_name::<unsafe extern "C" fn(u8, ...) -> u8>(),
    ]
}

fn values() -> Vec<&'static str> {
    vec![
        type_name_of_val(&Pair),
        type_name_of_val(&Wrapped::<u8>),
        type_name_of_val(&Either::<u8>::Left),
        type_name_of_val(&Some::<u8>),
        type_name_of_val(&async {}),
    ]
}

pub fn run() -> Vec<Vec<&'static str>> {
    vec![trait_objects(), function_pointers(), values()]
}
