//! Found by the fuzzer (`fuzz/found/differs-checksum-S1-1-c22.rs`): the order of drops when
//! an operand panics inside an aggregate that is itself an operand of another, as in
//! `(Noisy(27), Duo { a: Noisy(39), b: panics() })`. The operands are dropped in the reverse
//! of the order they were made, the inner one's first. ra-jit moved the operands of each
//! aggregate behind the temporaries when it was done with it, the outer after the inner,
//! which put the outer one's operands last and had them dropped first. The other parts are
//! panics in a destructor and in other shapes of operand, which were right.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

thread_local! { static LOG: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) }; }
fn feed(n: u32) { LOG.with(|it| it.borrow_mut().push(n)); }
struct Noisy(u32);
impl Drop for Noisy { fn drop(&mut self) { feed(1000 + self.0); } }
struct Bomb(u32);
impl Drop for Bomb { fn drop(&mut self) { feed(2000 + self.0); if !std::thread::panicking() { std::panic::panic_any(self.0); } } }
struct Duo<A, B> { a: A, b: B }
#[inline(never)] fn trip<T>(armed: bool, value: T) -> T { if armed { std::panic::panic_any(7u32); } value }
#[inline(never)] fn take3<A, B, C>(_a: A, _b: B, _c: C) {}
fn caught(f: impl FnOnce()) { feed(if catch_unwind(AssertUnwindSafe(f)).is_ok() { 1 } else { 2 }); }
fn part(name: u32, f: impl FnOnce()) -> Vec<u32> { feed(9000 + name); f(); LOG.with(|it| it.take()) }

pub fn run() -> Vec<Vec<u32>> {
    vec![
        part(1, || caught(|| { let _a = Noisy(18); let _ = Some(Bomb(54)); let _z = Noisy(55); })),
        part(2, || caught(|| { let _a = Noisy(18); let _h = (Noisy(1), Bomb(54), Noisy(2)); let _z = Noisy(55); })),
        part(3, || caught(|| { let _a = Noisy(18); drop(Box::new((Bomb(5), Noisy(6)))); let _z = Noisy(55); })),
        part(4, || caught(|| { let _a = Noisy(18); let mut v = vec![Bomb(1), Bomb(2), Bomb(3)]; v.truncate(1); let _z = Noisy(55); })),
        part(5, || for armed in [false, true] { caught(|| { let _v = (Noisy(27), Duo { a: Noisy(39), b: trip(armed, Noisy(15)) }); }); }),
        part(6, || for armed in [false, true] { caught(|| { let _v = { take3(Noisy(1), &Noisy(30), trip(armed, Noisy(15))) }; }); }),
        part(7, || for armed in [false, true] { caught(|| { let _v = Some(Noisy(31)).map(|it| (trip(armed, Noisy(29)), it)); }); }),
        part(8, || caught(|| { let _a = Noisy(18); { let _inner = [Some(Bomb(3)), None, Some(Bomb(4))]; let _late = Noisy(9); } let _z = Noisy(55); })),
        part(9, || caught(|| { let _a = Noisy(18); take3(Noisy(4), Duo { a: Bomb(7), b: Noisy(8) }, Noisy(5)); let _z = Noisy(55); })),
    ]
}
