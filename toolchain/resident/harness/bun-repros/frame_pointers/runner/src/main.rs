// bun's `bun_core::return_address`: `[fp + 8]` read in the frame of the function asking, which
// is sound only because bun builds with `-Cforce-frame-pointers=yes`. A function without a
// frame of its own reads its caller's frame pointer instead, or a register LLVM gave to
// something else (bun's `Entry::close_fd` faulted at 0x11).
use std::hint::black_box;

#[inline(never)]
fn returns_into_caller() -> usize {
    let frame: usize;
    // SAFETY: with frame pointers kept, the register holds this function's frame, and the
    // slot above it the address it returns to.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, rbp", out(reg) frame, options(nomem, nostack, preserves_flags));
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("mov {}, x29", out(reg) frame, options(nomem, nostack, preserves_flags));
        *((frame + 8) as *const usize)
    }
}

#[inline(never)]
fn caller(n: usize) -> (usize, usize) {
    let returns_to = returns_into_caller();
    (returns_to, black_box(n) * 2)
}

fn main() {
    let start = caller as fn(usize) -> (usize, usize) as usize;
    for n in 0..3 {
        let (returns_to, twice) = caller(n);
        println!(
            "{twice} returns into its caller: {}",
            (start..start + 4096).contains(&returns_to)
        );
    }
}
