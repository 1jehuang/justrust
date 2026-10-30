use std::sync::atomic::{AtomicUsize, Ordering};
static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Owned<T>(T);
impl<T> Drop for Owned<T> { fn drop(&mut self) { DROPS.fetch_add(1, Ordering::SeqCst); } }
#[inline(always)] fn leaf() -> u32 { 2 }
#[inline(always)] fn left() -> u32 { leaf() + 1 }
#[inline(always)] fn right() -> u32 { leaf() + 2 }
#[inline(always)] fn diamond() -> u32 { left() + right() }
#[inline(always)] fn cycle_a(n: u32) -> u32 { if n == 0 { 0 } else { cycle_b(n - 1) + 1 } }
#[inline(always)] fn cycle_b(n: u32) -> u32 { if n == 0 { 0 } else { cycle_a(n - 1) + 1 } }
#[inline(always)] fn generic<T: Copy>(value: T) -> T { value }
mod a {
    #[unsafe(no_mangle)] #[inline(never)] pub fn root_a() -> u32 { super::diamond() + 1 }
    #[unsafe(no_mangle)] #[inline(never)] pub fn root_b() -> u32 { super::diamond() + 2 }
}
mod c { #[unsafe(no_mangle)] #[inline(never)] pub fn root_c() -> u32 { super::diamond() + 3 } }
fn main() {
    { let _first = Owned(7u32); let _second = Owned(11u64); }
    println!("{} {} {} {} {} {} {}", a::root_a(), a::root_b(), c::root_c(),
             cycle_a(4), generic(7u32), generic(11u64), DROPS.load(Ordering::SeqCst));
}
