fn mapped(address: usize) -> bool {
    let mut status = 0u8;
    let output = &mut status as *mut u8 as usize;
    let result = unsafe { __justrust_linux_syscall6!(27usize, address, 4096usize, output, 0usize, 0usize, 0usize) };
    if result == 0usize { return true; }
    assert!(result == 18446744073709551604usize);
    false
}
fn identity<T>(value: T) -> T { value }
fn consume<T>(value: T, early: bool) {
    if early { return; }
    let moved = value;
}
fn through_break<T>(value: T) -> T { loop { break value; } }
fn main() {
    let address: usize;
    {
        let allocation = heap::allocate(4096usize, 4096usize);
        address = heap::pointer(&allocation) as usize;
        let moved = through_break::<heap::Allocation>(identity::<heap::Allocation>(allocation));
        assert!(mapped(address));
        assert!(heap::len(&moved) == 4096usize);
    }
    assert!(!mapped(address));
    let first = heap::allocate(4096usize,4096usize);
    let first_address = heap::pointer(&first) as usize;
    consume::<heap::Allocation>(first, true);
    assert!(!mapped(first_address));
    let second = heap::allocate(4096usize,4096usize);
    let second_address = heap::pointer(&second) as usize;
    consume::<heap::Allocation>(second, false);
    assert!(!mapped(second_address));
    let explicit = identity::<heap::Allocation>(heap::allocate(4096usize,4096usize));
    let explicit_address = heap::pointer(&explicit) as usize;
    assert!(heap::release(explicit));
    assert!(!mapped(explicit_address));
    println!("ok");
}
