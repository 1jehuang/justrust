//! A function whose only generic parameters are lifetimes is one function in rustc's build,
//! and ra-jit compiled a copy of its own for every call instead of calling rustc's.
//! `DebugTuple<'a, 'b>::field` and `DebugList::entry` are such, so printing a nested value
//! went through seventeen unoptimised frames a level where rustc's build has seven, and
//! simd-json's `limit_fits_in_a_default_thread_stack` (1024 levels in 2 MiB) overflowed.

#[derive(Debug)]
enum Value {
    Leaf,
    Array(Vec<Value>),
}

pub fn run() -> usize {
    let printed = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut value = Value::Leaf;
            for _ in 0..1024 {
                value = Value::Array(vec![value]);
            }
            let printed = format!("{value:?}");
            // Dropping it recurses as well, and is not what is measured.
            std::mem::forget(value);
            printed
        })
        .unwrap()
        .join()
        .unwrap();
    printed.len()
}
