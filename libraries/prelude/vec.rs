// Native Vec<T>: a growable heap buffer written in ordinary Rust source.
use super::mem::{raw_alloc, raw_free, size_of};
use super::option::Option;

pub struct Vec<T> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}

fn slot<T>(base: *mut T, index: usize) -> *mut T {
    (base as usize + index * size_of::<T>()) as *mut T
}

impl<T> Vec<T> {
    pub fn new() -> Vec<T> {
        Vec {
            ptr: 0usize as *mut T,
            len: 0usize,
            cap: 0usize,
        }
    }

    pub fn with_capacity(capacity: usize) -> Vec<T> {
        let mut v = Vec { ptr: 0usize as *mut T, len: 0usize, cap: 0usize };
        v.reserve(capacity);
        v
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0usize
    }

    pub fn as_ptr(&self) -> *const T {
        self.ptr as *const T
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr
    }

    pub fn reserve(&mut self, additional: usize) {
        let needed = self.len + additional;
        if needed <= self.cap {
            return;
        }
        let mut cap = if self.cap == 0usize { 4usize } else { self.cap * 2usize };
        if cap < needed {
            cap = needed;
        }
        let size = size_of::<T>();
        let bytes = cap * size;
        let fresh = raw_alloc(if bytes == 0usize { 8usize } else { bytes }) as *mut T;
        let mut i = 0usize;
        while i < self.len {
            let src = slot(self.ptr, i);
            let dst = slot(fresh, i);
            unsafe {
                *dst = *src;
            }
            i += 1usize;
        }
        raw_free(self.ptr as usize, self.cap * size);
        self.ptr = fresh;
        self.cap = cap;
    }

    pub fn push(&mut self, value: T) {
        if self.len == self.cap {
            self.reserve(1usize);
        }
        let dst = slot(self.ptr, self.len);
        unsafe {
            *dst = value;
        }
        self.len += 1usize;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0usize {
            return Option::None;
        }
        self.len -= 1usize;
        let src = slot(self.ptr, self.len);
        Option::Some(unsafe { *src })
    }

    pub fn insert(&mut self, index: usize, value: T) {
        if index > self.len {
            panic!("insertion index out of bounds");
        }
        if self.len == self.cap {
            self.reserve(1usize);
        }
        let mut i = self.len;
        while i > index {
            let src = slot(self.ptr, i - 1usize);
            let dst = slot(self.ptr, i);
            unsafe {
                *dst = *src;
            }
            i -= 1usize;
        }
        let dst = slot(self.ptr, index);
        unsafe {
            *dst = value;
        }
        self.len += 1usize;
    }

    pub fn remove(&mut self, index: usize) -> T {
        if index >= self.len {
            panic!("removal index out of bounds");
        }
        let src = slot(self.ptr, index);
        let value = unsafe { *src };
        let mut i = index;
        while i + 1usize < self.len {
            let from = slot(self.ptr, i + 1usize);
            let to = slot(self.ptr, i);
            unsafe {
                *to = *from;
            }
            i += 1usize;
        }
        self.len -= 1usize;
        value
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        if index >= self.len {
            panic!("swap_remove index out of bounds");
        }
        let at = slot(self.ptr, index);
        let value = unsafe { *at };
        self.len -= 1usize;
        if index != self.len {
            let last = slot(self.ptr, self.len);
            unsafe {
                *at = *last;
            }
        }
        value
    }

    pub fn truncate(&mut self, len: usize) {
        while self.len > len {
            self.len -= 1usize;
            let src = slot(self.ptr, self.len);
            let _dropped = unsafe { *src };
        }
    }

    pub fn clear(&mut self) {
        self.truncate(0usize);
    }
}

impl<T> Drop for Vec<T> {
    fn drop(&mut self) {
        self.truncate(0usize);
        raw_free(self.ptr as usize, self.cap * size_of::<T>());
        self.ptr = 0usize as *mut T;
        self.cap = 0usize;
    }
}
