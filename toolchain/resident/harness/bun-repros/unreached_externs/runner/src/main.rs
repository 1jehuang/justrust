// bun's `main` is the C runtime's, `#[no_mangle]` in `bun_runtime`: with no `main` of Rust's,
// ra-jit compiles every function of the crates, as rustc does, and the link keeps what `main`
// reaches.
#![no_main]

#[unsafe(no_mangle)]
extern "C" fn main(_argc: core::ffi::c_int, _argv: *const *const u8) -> core::ffi::c_int {
    println!("{}", unreached_externs::greeting());
    0
}
