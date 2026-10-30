//! tests/ui/abi/statics/static-mut-foreign.rs: a static declared in an `extern` block is
//! defined by a library the program links, under the name it is declared with. ra-jit looked
//! for it among the symbols rustc made, found none and refused the function.

#[cfg(target_os = "linux")]
#[inline(never)]
fn has_environment() -> bool {
    unsafe extern "C" {
        static environ: *const *const u8;
        #[link_name = "environ"]
        static renamed: *const *const u8;
    }
    unsafe { !environ.is_null() && environ == renamed }
}

#[cfg(not(target_os = "linux"))]
fn has_environment() -> bool {
    true
}

pub fn run() -> bool {
    has_environment()
}
