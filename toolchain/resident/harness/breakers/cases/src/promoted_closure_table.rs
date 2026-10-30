//! A table of closures that capture nothing, coerced to `fn` and borrowed in a generic
//! function: rustc promotes it to a constant, as minijinja's `type_erase!` builds
//! `DynObject`'s vtable. ra-jit took it as a temporary, so the reference outlived the frame
//! (the borrow checker said E0515, and without it the call through the table jumped to 0).

struct Table {
    add: fn(u32) -> u32,
    name: fn() -> &'static str,
}

fn table<T: Default + Into<u32>>() -> &'static Table {
    &Table { add: |x| x + T::default().into() + 1, name: || std::any::type_name::<T>() }
}

pub fn run() -> (u32, &'static str, u32, &'static str) {
    let (small, wide) = (table::<u8>(), table::<u32>());
    ((small.add)(41), (small.name)(), (wide.add)(1), (wide.name)())
}
