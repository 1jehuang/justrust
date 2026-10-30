// The C runtime's `main`: with no `main` of Rust's, ra-jit compiles every function of the
// crates it is given, as it did all of bun's.
#![no_main]

#[unsafe(no_mangle)]
extern "C" fn main(_argc: core::ffi::c_int, _argv: *const *const u8) -> core::ffi::c_int {
    println!("{}", same_name::greeting());
    0
}
