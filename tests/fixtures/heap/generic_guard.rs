struct Guard<T> { pointer: *mut T, count: *mut u64 }
impl<T> Drop for Guard<T> {
    fn drop(&mut self) { unsafe { *self.count = *self.count + 1u64; } }
}
fn identity<T>(value: T) -> T { value }
fn make<T>(pointer: *mut T, count: *mut u64) -> Guard<T> { Guard::<T> { pointer: pointer, count: count } }
fn main() {
    let mut count = 0u64;
    let counter = &mut count as *mut u64;
    let mut first = 12i32;
    let mut second = 7u8;
    {
        let first_guard = identity::<Guard<i32>>(make::<i32>(&mut first as *mut i32, counter));
        let second_guard = make::<u8>(&mut second as *mut u8, counter);
        unsafe { *first_guard.pointer = 13i32; *second_guard.pointer = 8u8; }
        assert!(first == 13i32);
        assert!(second == 8u8);
        assert!(count == 0u64);
    }
    assert!(count == 2u64);
    println!("ok");
}
