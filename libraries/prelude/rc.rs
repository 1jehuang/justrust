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

/// Interior mutability with a dynamic borrow flag: 0 free, n>0 shared
/// readers, usize::MAX one writer. Conflicts panic, as std's do. Guards hold
/// raw pointers; the `'b` borrow of the cell is carried by `life`.
pub struct RefCell<T> {
    flag: usize,
    value: T,
}

pub struct Ref<'b, T> {
    flag: *mut usize,
    value: *const T,
    life: *const &'b (),
}

pub struct RefMut<'b, T> {
    flag: *mut usize,
    value: *mut T,
    life: *const &'b (),
}

impl<T> RefCell<T> {
    pub fn new(value: T) -> RefCell<T> {
        RefCell { flag: 0usize, value }
    }

    pub fn borrow<'b>(&'b self) -> Ref<'b, T> {
        let flag = &self.flag as *const usize as *mut usize;
        unsafe {
            if *flag == 18446744073709551615usize {
                panic!("already mutably borrowed: BorrowError");
            }
            *flag += 1usize;
        }
        let value = &self.value as *const T;
        Ref {
            flag,
            value,
            life: 0usize as *const &'b (),
        }
    }

    pub fn borrow_mut<'b>(&'b self) -> RefMut<'b, T> {
        let flag = &self.flag as *const usize as *mut usize;
        unsafe {
            if *flag != 0usize {
                panic!("already borrowed: BorrowMutError");
            }
            *flag = 18446744073709551615usize;
        }
        let value = &self.value as *const T as *mut T;
        RefMut {
            flag,
            value,
            life: 0usize as *const &'b (),
        }
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<'b, T> super::ops::Deref for Ref<'b, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.value }
    }
}

impl<'b, T> Drop for Ref<'b, T> {
    fn drop(&mut self) {
        unsafe {
            *self.flag -= 1usize;
        }
    }
}

impl<'b, T> super::ops::Deref for RefMut<'b, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.value }
    }
}

impl<'b, T> super::ops::DerefMut for RefMut<'b, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.value }
    }
}

impl<'b, T> Drop for RefMut<'b, T> {
    fn drop(&mut self) {
        unsafe {
            *self.flag = 0usize;
        }
    }
}

impl<'b, T: super::fmt::Debug> super::fmt::Debug for Ref<'b, T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        unsafe { super::fmt::Debug::fmt(&*self.value, f) }
    }
}

impl<'b, T: super::fmt::Display> super::fmt::Display for Ref<'b, T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        unsafe { super::fmt::Display::fmt(&*self.value, f) }
    }
}

impl<'b, T: super::fmt::Debug> super::fmt::Debug for RefMut<'b, T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        unsafe { super::fmt::Debug::fmt(&*self.value, f) }
    }
}

impl<'b, T: super::fmt::Display> super::fmt::Display for RefMut<'b, T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        unsafe { super::fmt::Display::fmt(&*self.value, f) }
    }
}
