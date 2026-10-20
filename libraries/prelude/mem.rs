// std::mem and the bootstrap allocator, as ordinary Rust source.
// size_of uses the address distance between two adjacent array elements at
// a fake non-null address; no memory is accessed.

pub fn size_of<T>() -> usize {
    let base = 4096usize as *const [T; 2];
    let second = unsafe { &(*base)[1] } as *const T;
    second as usize - 4096usize
}

// Compiler-owned value carrier extent, not Rust's public size_of.
pub(super) fn storage_stride<T>() -> usize {
    __justrust_storage_stride!(T)
}

// Packed native bytes are not the compiler's private u8 carrier slots.
// This helper remains ordinary source; its one query has separate exact
// original-function authority. No caller or allocator effect is privileged.
pub(super) fn packed_reserve(address: usize, old_count: usize, new_count: usize) -> usize {
    let unit = __justrust_storage_stride!(u8);
    if new_count == 0usize {
        return 0usize;
    }
    if old_count > 18446744073709551615usize / unit || new_count > 18446744073709551615usize / unit
    {
        return 0usize;
    }
    raw_realloc(address, old_count * unit, new_count * unit)
}

pub fn align_of<T>() -> usize {
    __justrust_align_of!(T)
}

pub fn swap<T>(a: &mut T, b: &mut T) {
    let pa = a as *mut T;
    let pb = b as *mut T;
    unsafe {
        let tmp = *pa;
        *pa = *pb;
        *pb = tmp;
    }
}

pub fn replace<T>(dest: &mut T, src: T) -> T {
    let p = dest as *mut T;
    unsafe {
        let old = *p;
        *p = src;
        old
    }
}

// Raw page allocation. Returns null on failure. Bytes are zeroed.
pub fn raw_alloc(bytes: usize) -> usize {
    if bytes == 0usize {
        return 0usize;
    }
    let result = unsafe {
        __justrust_linux_syscall6!(
            9usize,
            0usize,
            bytes,
            3usize,
            34usize,
            18446744073709551615usize,
            0usize
        )
    };
    if result >= 18446744073709547521usize {
        return 0usize;
    }
    result
}

pub fn raw_free(address: usize, bytes: usize) {
    if address == 0usize || bytes == 0usize {
        return;
    }
    let _ = unsafe {
        __justrust_linux_syscall6!(11usize, address, bytes, 0usize, 0usize, 0usize, 0usize)
    };
}

// Resize an owned raw mapping. Linux preserves the complete old prefix even
// when MREMAP_MAYMOVE relocates it. Failure leaves the original mapping live.
// Like raw_alloc, a zero result denotes allocation failure, except that a
// zero-sized request consumes the old mapping and intentionally returns zero.
pub fn raw_realloc(address: usize, old_bytes: usize, new_bytes: usize) -> usize {
    if new_bytes == 0usize {
        raw_free(address, old_bytes);
        return 0usize;
    }
    if address == 0usize || old_bytes == 0usize {
        return raw_alloc(new_bytes);
    }
    let result = unsafe {
        __justrust_linux_syscall6!(25usize, address, old_bytes, new_bytes, 1usize, 0usize, 0usize)
    };
    if result >= 18446744073709547521usize {
        return 0usize;
    }
    result
}

// Write a runtime panic message to stderr. Callers follow it with `panic!`,
// which reports the location and exits with the panic status.
pub fn panic_message(msg: &str) {
    let (p, n) = super::string::str_parts(msg);
    let _ = unsafe { __justrust_linux_syscall6!(1usize, 2usize, p, n, 0usize, 0usize, 0usize) };
    let _ = unsafe {
        __justrust_linux_syscall6!(1usize, 2usize, super::string::str_parts("\n").0, 1usize, 0usize, 0usize, 0usize)
    };
}

// Backing storage for `static mut` items. Each static owns a fixed 16 MiB
// address window derived from its id. The first access maps it
// (MAP_FIXED_NOREPLACE) and writes the initializer; later accesses observe
// EEXIST and reuse the live value.
pub fn static_slot<T>(id: usize, init: T) -> *mut T {
    let address = 35184372088832usize + id * 16777216usize;
    let size = size_of::<T>();
    let bytes = if size == 0usize { 4096usize } else { (size + 4095usize) / 4096usize * 4096usize };
    let result = unsafe {
        __justrust_linux_syscall6!(9usize, address, bytes, 3usize, 1048610usize, 18446744073709551615usize, 0usize)
    };
    let p = address as *mut T;
    if result == address {
        unsafe { *p = init; }
    }
    p
}
