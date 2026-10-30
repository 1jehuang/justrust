#![no_std]
#![no_main]

use core::fmt::Write;

#[cfg(not(windows))]
#[link(name = "c")]
extern "C" {
    fn write(fd: i32, bytes: *const u8, len: usize) -> isize;
    fn exit(status: i32) -> !;
}

/// The C runtime of MSVC, which is also what calls `main`.
#[cfg(windows)]
#[link(name = "msvcrt")]
#[link(name = "ucrt")]
#[link(name = "vcruntime")]
extern "C" {
    #[link_name = "_write"]
    fn write(fd: i32, bytes: *const u8, len: u32) -> i32;
    fn exit(status: i32) -> !;
}

/// What the unwinder would ask of a frame: the prebuilt `core` names it, nothing calls it.
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

struct Out;

impl Write for Out {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        unsafe { write(1, text.as_ptr(), text.len() as _) };
        Ok(())
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    let _ = writeln!(Out, "panicked");
    unsafe { exit(3) }
}

/// Dropped when the function that holds it returns, and, were panics to unwind, when a call
/// it is held across panics: with `panic = "abort"` none does, and nothing calls the unwinder.
struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = writeln!(Out, "dropped {}", self.0);
    }
}

#[inline(never)]
fn doubled(n: u32) -> u32 {
    if n > 1000 {
        panic!("too large");
    }
    n * 2
}

fn held(n: u32) -> u32 {
    let guard = Guard(n);
    let doubled = doubled(n) + doubled(guard.0 + 1);
    let _ = writeln!(Out, "holding {}", guard.0);
    doubled
}

#[no_mangle]
pub extern "C" fn main(argc: i32, _argv: *const *const u8) -> i32 {
    let _ = writeln!(Out, "no_std: {}", held(argc as u32));
    0
}
