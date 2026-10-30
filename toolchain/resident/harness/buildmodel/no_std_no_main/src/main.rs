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
    unsafe { exit(3) }
}

fn checked(a: u32, b: u32) -> Option<u32> {
    a.checked_mul(b)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, _argv: *const *const u8) -> i32 {
    let _ = writeln!(Out, "no_std: {} {:?}", argc, checked(argc as u32, 21));
    0
}
