// Rc owns separate typed value and count allocations. Pointee unsizing preserves
// both allocations, without unsized inline-header layout or base reconstruction.
use super::mem::{raw_alloc, raw_free, storage_stride};
use core::cell::Cell;

pub struct Rc<T: ?Sized> {
    ptr: *mut T,
    strong: *mut usize,
}

impl<T> Rc<T> {
    pub fn new(value: T) -> Rc<T> {
        let bytes = storage_stride::<T>();
        let bytes = if bytes == 0usize { 8usize } else { bytes };
        let p = raw_alloc(bytes) as *mut T;
        if p as usize == 0usize {
            panic!("memory allocation failed");
        }
        let strong = raw_alloc(storage_stride::<usize>()) as *mut usize;
        if strong as usize == 0usize {
            raw_free(p as usize, bytes);
            panic!("memory allocation failed");
        }
        unsafe {
            *p = value;
            *strong = 1usize;
        }
        Rc { ptr: p, strong }
    }
}

impl<T: ?Sized> Rc<T> {
    pub fn strong_count(this: &Rc<T>) -> usize {
        unsafe { *this.strong }
    }

    pub fn ptr_eq(this: &Rc<T>, other: &Rc<T>) -> bool {
        // Each live value allocation has exactly one control allocation.
        // Compare those thin identities, not an erased fat-pointer cast.
        this.strong as usize == other.strong as usize
    }
}

impl<T: ?Sized> Clone for Rc<T> {
    fn clone(&self) -> Rc<T> {
        let strong = self.strong;
        unsafe {
            if *strong == 18446744073709551615usize {
                super::process::abort();
            }
            *strong += 1usize;
        }
        Rc { ptr: self.ptr, strong }
    }
}

impl<T: ?Sized> Drop for Rc<T> {
    fn drop(&mut self) {
        // Exact reserved-carrier cleanup is sealed by normal_drop_plan. It
        // decrements strong, drops T in its original allocation at zero,
        // and releases both allocations. Moving T into a local here would
        // change the address observed by its destructor.
    }
}

impl<T: ?Sized> super::ops::Deref for Rc<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.ptr }
    }
}

/// Interior mutability with a dynamic borrow flag: 0 free, n>0 shared
/// readers, usize::MAX one writer. Conflicts panic, as std's do. Guards hold
/// raw value pointers and a shared reference to the scalar borrow counter.
/// The counter reference keeps the cell alive through ordinary carrier analysis.
pub struct RefCell<T> {
    flag: Cell<usize>,
    value: T,
}

pub struct Ref<'b, T> {
    flag: &'b Cell<usize>,
    value: *const T,
}

pub struct RefMut<'b, T> {
    flag: &'b Cell<usize>,
    value: *mut T,
}

pub struct BorrowError {
    private: (),
}

pub struct BorrowMutError {
    private: (),
}

impl super::fmt::Debug for BorrowError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad("BorrowError")
    }
}

impl super::fmt::Display for BorrowError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad("RefCell already mutably borrowed")
    }
}

impl super::fmt::Debug for BorrowMutError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad("BorrowMutError")
    }
}

impl super::fmt::Display for BorrowMutError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad("RefCell already borrowed")
    }
}

impl<T> RefCell<T> {
    pub fn new(value: T) -> RefCell<T> {
        RefCell { flag: Cell::new(0usize), value }
    }

    pub fn borrow<'b>(&'b self) -> Ref<'b, T> {
        match self.try_borrow() {
            Ok(guard) => guard,
            Err(_) => panic!("already mutably borrowed: BorrowError"),
        }
    }

    pub fn try_borrow<'b>(&'b self) -> Result<Ref<'b, T>, BorrowError> {
        let readers = self.flag.get();
        // Match the signed borrow-counter reader limit while reserving
        // usize::MAX for the writer. Failure never changes the flag.
        if readers >= 9223372036854775807usize {
            return Err(BorrowError { private: () });
        }
        self.flag.set(readers + 1usize);
        let value = &self.value as *const T;
        Ok(Ref {
            flag: &self.flag,
            value,
        })
    }

    pub fn borrow_mut<'b>(&'b self) -> RefMut<'b, T> {
        match self.try_borrow_mut() {
            Ok(guard) => guard,
            Err(_) => panic!("already borrowed: BorrowMutError"),
        }
    }

    pub fn try_borrow_mut<'b>(&'b self) -> Result<RefMut<'b, T>, BorrowMutError> {
        if self.flag.get() != 0usize {
            return Err(BorrowMutError { private: () });
        }
        self.flag.set(18446744073709551615usize);
        let value = &self.value as *const T as *mut T;
        Ok(RefMut {
            flag: &self.flag,
            value,
        })
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
        self.flag.set(self.flag.get() - 1usize);
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
        self.flag.set(0usize);
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
