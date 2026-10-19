//! Bootstrap Linux heap policy, implemented as ordinary Rust source.
//! Not std::alloc compatibility. Layouts are nonzero, power-of-two aligned,
//! at most 4096-byte aligned, and at most isize::MAX bytes. Invalid layouts
//! and kernel allocation failures produce a null allocation handle.
//! Allocation owns a distinct anonymous mapping. Release consumes the handle.
//! No allocator state, metadata table, or compiler collection special cases.

pub struct Allocation {
    address: *mut u8,
    size: usize,
}

pub fn null<T>() -> *mut T {
    0usize as *mut T
}

pub fn is_null<T>(pointer: *const T) -> bool {
    pointer as usize == 0usize
}

// Integer-address arithmetic does not assert in-bounds provenance. A caller
// must establish allocation bounds and alignment before unsafe dereference.
// Overflow/underflow returns null rather than wrapping to a different address.
pub fn byte_add(pointer: *mut u8, count: usize) -> *mut u8 {
    let address = pointer as usize;
    if count > 18446744073709551615usize - address {
        return null::<u8>();
    }
    (address + count) as *mut u8
}

pub fn byte_sub(pointer: *mut u8, count: usize) -> *mut u8 {
    let address = pointer as usize;
    if count > address {
        return null::<u8>();
    }
    (address - count) as *mut u8
}

pub fn valid_layout(size: usize, align: usize) -> bool {
    size != 0usize && size <= 9223372036854775807usize
        && align != 0usize && align <= 4096usize
        && (align & (align - 1usize)) == 0usize
}

pub fn allocate(size: usize, align: usize) -> Allocation {
    if !valid_layout(size, align) {
        return Allocation { address: null::<u8>(), size: 0usize };
    }
    // mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0).
    // Linux x86-64 mmap addresses are at least 4096-byte aligned.
    let result = unsafe {
        __justrust_linux_syscall6!(9usize, 0usize, size, 3usize, 34usize,
            18446744073709551615usize, 0usize)
    };
    // Raw kernel errors occupy the unsigned representations of -4095..=-1.
    if result >= 18446744073709547521usize {
        return Allocation { address: null::<u8>(), size: 0usize };
    }
    Allocation { address: result as *mut u8, size }
}

pub fn pointer(allocation: &Allocation) -> *mut u8 {
    allocation.address
}

pub fn len(allocation: &Allocation) -> usize {
    allocation.size
}

pub fn release(allocation: Allocation) -> bool {
    if is_null::<u8>(allocation.address as *const u8) {
        return true;
    }
    // Fields are private and Allocation is not Copy. Only allocate constructs
    // a live handle, so this safe consuming API cannot unmap arbitrary memory.
    let result = unsafe {
        __justrust_linux_syscall6!(11usize, allocation.address as usize,
            allocation.size, 0usize, 0usize, 0usize, 0usize)
    };
    result == 0usize
}
