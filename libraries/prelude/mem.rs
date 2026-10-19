// std::mem and the bootstrap allocator, as ordinary Rust source.
// size_of uses the address distance between two adjacent array elements at
// a fake non-null address; no memory is accessed.

pub fn size_of<T>() -> usize {
    let base = 4096usize as *const [T; 2];
    let second = unsafe { &(*base)[1] } as *const T;
    second as usize - 4096usize
}

pub fn align_of<T>() -> usize {
    8usize
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

// Write a runtime panic message to stderr. Callers follow it with `panic!`,
// which reports the location and exits with the panic status.
pub fn panic_message(msg: &str) {
    let (p, n) = super::string::str_parts(msg);
    let _ = unsafe { __justrust_linux_syscall6!(1usize, 2usize, p, n, 0usize, 0usize, 0usize) };
    let _ = unsafe {
        __justrust_linux_syscall6!(1usize, 2usize, super::string::str_parts("\n").0, 1usize, 0usize, 0usize, 0usize)
    };
}
