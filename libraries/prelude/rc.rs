// Rc<T> and RefCell<T> / Cell<T> as ordinary Rust source. Single-threaded
// shared ownership: one heap block holds the strong count and the value.
use super::mem::{raw_alloc, raw_free, storage_stride};

struct RcBox<T> {
    strong: usize,
    value: T,
}

pub struct Rc<T> {
    ptr: *mut RcBox<T>,
}

impl<T> Rc<T> {
    pub fn new(value: T) -> Rc<T> {
        let bytes = storage_stride::<RcBox<T>>();
        let p = raw_alloc(if bytes == 0usize { 8usize } else { bytes }) as *mut RcBox<T>;
        if p as usize == 0usize {
            panic!("memory allocation failed");
        }
        unsafe {
            *p = RcBox { strong: 1usize, value };
        }
        Rc { ptr: p }
    }

    pub fn strong_count(this: &Rc<T>) -> usize {
        unsafe { (*this.ptr).strong }
    }

    pub fn ptr_eq(this: &Rc<T>, other: &Rc<T>) -> bool {
        this.ptr as usize == other.ptr as usize
    }
}

impl<T> Clone for Rc<T> {
    fn clone(&self) -> Rc<T> {
        let p = self.ptr;
        unsafe {
            (*p).strong += 1usize;
        }
        Rc { ptr: p }
    }
}

impl<T> Drop for Rc<T> {
    fn drop(&mut self) {
        let p = self.ptr;
        let remaining = unsafe {
            (*p).strong -= 1usize;
            (*p).strong
        };
        if remaining == 0usize {
            let _inner = unsafe { *p };
            let bytes = storage_stride::<RcBox<T>>();
            raw_free(p as usize, if bytes == 0usize { 8usize } else { bytes });
        }
    }
}

impl<T> super::ops::Deref for Rc<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &(*self.ptr).value }
    }
}

