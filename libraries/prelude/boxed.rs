// Box<T>: a single owning heap pointer, as ordinary Rust source.
// Layout is exactly one `*mut T` word (unsized Box<dyn T> becomes data+vtable).
use super::mem::{raw_alloc, raw_free, storage_stride};

pub struct Box<T: ?Sized> {
    ptr: *mut T,
}

impl<T> Box<T> {
    pub fn new(value: T) -> Box<T> {
        let bytes = storage_stride::<T>();
        let p = raw_alloc(if bytes == 0usize { 8usize } else { bytes }) as *mut T;
        if p as usize == 0usize {
            panic!("memory allocation failed");
        }
        unsafe {
            *p = value;
        }
        Box { ptr: p }
    }

    pub fn into_inner(b: Box<T>) -> T {
        let p = b.ptr;
        let value = unsafe { *p };
        let bytes = storage_stride::<T>();
        raw_free(p as usize, if bytes == 0usize { 8usize } else { bytes });
        // The value moved out; skip the destructor's pointee drop.
        let mut b = b;
        b.ptr = 0usize as *mut T;
        value
    }
}

// Borrowing the pointee needs no size: `Box<dyn Trait>::as_mut` is `&mut dyn Trait`.
impl<T: ?Sized> Box<T> {
    pub fn as_ref(&self) -> &T {
        unsafe { &*self.ptr }
    }

    pub fn as_mut(&mut self) -> &mut T {
        unsafe { &mut *self.ptr }
    }
}

impl<T: ?Sized> Drop for Box<T> {
    fn drop(&mut self) {
        // Exact reserved-carrier cleanup is sealed by normal_drop_plan. It
        // drops T in its original allocation, then frees the private storage
        // extent (from the closed type or the dynamic vtable). The null
        // pointer left by into_inner suppresses both actions. This source
        // declaration covers the same ?Sized domain as Box itself.
    }
}

impl<T: ?Sized> super::ops::Deref for Box<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.ptr }
    }
}

impl<T: ?Sized> super::ops::DerefMut for Box<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.ptr }
    }
}

impl<T: super::fmt::Debug + ?Sized> super::fmt::Debug for Box<T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        <T as super::fmt::Debug>::fmt(unsafe { &*self.ptr }, f)
    }
}

impl<T: super::fmt::Display + ?Sized> super::fmt::Display for Box<T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        <T as super::fmt::Display>::fmt(unsafe { &*self.ptr }, f)
    }
}
