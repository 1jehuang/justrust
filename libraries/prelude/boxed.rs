// Box<T>: a single owning heap pointer, as ordinary Rust source.
// Layout is exactly one `*mut T` word (unsized Box<dyn T> becomes data+vtable).
use super::mem::{raw_alloc, raw_free, size_of};

pub struct Box<T> {
    ptr: *mut T,
}

impl<T> Box<T> {
    pub fn new(value: T) -> Box<T> {
        let bytes = size_of::<T>();
        let p = raw_alloc(if bytes == 0usize { 8usize } else { bytes }) as *mut T;
        if p as usize == 0usize {
            panic!("memory allocation failed");
        }
        unsafe {
            *p = value;
        }
        Box { ptr: p }
    }

    pub fn as_ref(&self) -> &T {
        unsafe { &*self.ptr }
    }

    pub fn as_mut(&mut self) -> &mut T {
        unsafe { &mut *self.ptr }
    }

    pub fn into_inner(b: Box<T>) -> T {
        let p = b.ptr;
        let value = unsafe { *p };
        let bytes = size_of::<T>();
        raw_free(p as usize, if bytes == 0usize { 8usize } else { bytes });
        // The value moved out; skip the destructor's pointee drop.
        let mut b = b;
        b.ptr = 0usize as *mut T;
        value
    }
}

impl<T> Drop for Box<T> {
    fn drop(&mut self) {
        let p = self.ptr;
        if p as usize == 0usize {
            return;
        }
        let _value = unsafe { *p };
        let bytes = size_of::<T>();
        raw_free(p as usize, if bytes == 0usize { 8usize } else { bytes });
    }
}
