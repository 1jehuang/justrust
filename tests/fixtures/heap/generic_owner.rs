struct Owner { count: *mut u64 }
impl Drop for Owner {
    fn drop(&mut self) { unsafe { *self.count = *self.count + 1u64; } }
}
fn identity<T>(value: T) -> T { value }
fn consume<T>(value: T) { }
fn choose<T>(value: T, early: bool) -> T {
    if early { return value; }
    identity::<T>(value)
}
fn replace<T>(left: T, right: T) -> T { let mut value = left; value = right; value }
fn branch<T>(value: T, moved: bool) {
    if moved { consume::<T>(value); }
}
fn main() {
    let mut count = 0u64;
    let pointer = &mut count as *mut u64;
    consume::<i32>(11i32);
    assert!(count == 0u64);
    { let value = identity::<Owner>(Owner { count: pointer }); }
    assert!(count == 1u64);
    consume::<Owner>(Owner { count: pointer });
    assert!(count == 2u64);
    { let value = choose::<Owner>(Owner { count: pointer }, true); }
    assert!(count == 3u64);
    { let value = choose::<Owner>(Owner { count: pointer }, false); }
    assert!(count == 4u64);
    branch::<Owner>(Owner { count: pointer }, true);
    assert!(count == 5u64);
    branch::<Owner>(Owner { count: pointer }, false);
    assert!(count == 6u64);
    { let value = replace::<Owner>(Owner { count: pointer }, Owner { count: pointer }); assert!(count == 7u64); }
    assert!(count == 8u64);
    println!("ok");
}
