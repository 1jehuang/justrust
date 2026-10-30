//! library/coretests ptr.rs, `test_ptr_default`: a raw pointer is null by default, which
//! `core` says for any `T: ?Sized + Thin`. `Thin` is a trait alias, and ra-jit asked that an
//! alias hold of the type before its own bounds could show that it does, so the impl was never
//! found and the call was left to rustc.

#[derive(Default)]
struct Held {
    shared: *const u64,
    unique: *mut [u8; 3],
}

#[inline(never)]
fn nulls() -> (bool, bool, bool) {
    let held = Held::default();
    let bare: *mut u8 = Default::default();
    (held.shared.is_null(), held.unique.is_null(), bare.is_null())
}

pub fn run() -> (bool, bool, bool) {
    nulls()
}
