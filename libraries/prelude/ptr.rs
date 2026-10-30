// std::ptr and the raw pointer methods, as ordinary Rust source. The pointer
// methods are trait methods of `ConstPtr`/`MutPtr` (injected by the prelude)
// rather than compiler-intrinsic inherent methods. Address arithmetic is
// plain integer arithmetic with an authenticated supported pointer stride.

pub fn null<T>() -> *const T {
    0usize as *const T
}

pub fn null_mut<T>() -> *mut T {
    0usize as *mut T
}

pub unsafe fn read<T>(src: *const T) -> T {
    let p = src as *mut T;
    unsafe { *p }
}

pub unsafe fn write<T>(dst: *mut T, value: T) {
    unsafe {
        *dst = value;
    }
}

pub fn eq<T>(a: *const T, b: *const T) -> bool {
    a as usize == b as usize
}

fn step<T>(address: usize, count: usize) -> usize {
    address + count * super::mem::pointer_stride::<T>()
}

fn back<T>(address: usize, count: usize) -> usize {
    address - count * super::mem::pointer_stride::<T>()
}

fn signed<T>(address: usize, count: isize) -> usize {
    if count < 0isize {
        back::<T>(address, (0isize - count) as usize)
    } else {
        step::<T>(address, count as usize)
    }
}

fn wrapping_signed<T>(address: usize, count: isize) -> usize {
    let size = super::mem::pointer_stride::<T>() as isize;
    address.wrapping_add(count.wrapping_mul(size) as usize)
}

pub trait ConstPtr<T> {
    fn is_null(self) -> bool;
    unsafe fn add(self, count: usize) -> *const T;
    unsafe fn sub(self, count: usize) -> *const T;
    unsafe fn offset(self, count: isize) -> *const T;
    fn wrapping_add(self, count: usize) -> *const T;
    fn wrapping_sub(self, count: usize) -> *const T;
    fn wrapping_offset(self, count: isize) -> *const T;
    unsafe fn offset_from(self, origin: *const T) -> isize;
    unsafe fn read(self) -> T;
    fn addr(self) -> usize;
}

impl<T> ConstPtr<T> for *const T {
    fn is_null(self) -> bool {
        self as usize == 0usize
    }
    unsafe fn add(self, count: usize) -> *const T {
        step::<T>(self as usize, count) as *const T
    }
    unsafe fn sub(self, count: usize) -> *const T {
        back::<T>(self as usize, count) as *const T
    }
    unsafe fn offset(self, count: isize) -> *const T {
        signed::<T>(self as usize, count) as *const T
    }
    fn wrapping_add(self, count: usize) -> *const T {
        wrapping_signed::<T>(self as usize, count as isize) as *const T
    }
    fn wrapping_sub(self, count: usize) -> *const T {
        wrapping_signed::<T>(self as usize, 0isize.wrapping_sub(count as isize)) as *const T
    }
    fn wrapping_offset(self, count: isize) -> *const T {
        wrapping_signed::<T>(self as usize, count) as *const T
    }
    unsafe fn offset_from(self, origin: *const T) -> isize {
        ((self as usize).wrapping_sub(origin as usize) as isize) / (super::mem::pointer_stride::<T>() as isize)
    }
    unsafe fn read(self) -> T {
        let p = self as *mut T;
        unsafe { *p }
    }
    fn addr(self) -> usize {
        self as usize
    }
}

pub trait MutPtr<T> {
    fn is_null(self) -> bool;
    unsafe fn add(self, count: usize) -> *mut T;
    unsafe fn sub(self, count: usize) -> *mut T;
    unsafe fn offset(self, count: isize) -> *mut T;
    fn wrapping_add(self, count: usize) -> *mut T;
    fn wrapping_sub(self, count: usize) -> *mut T;
    fn wrapping_offset(self, count: isize) -> *mut T;
    unsafe fn offset_from(self, origin: *const T) -> isize;
    unsafe fn read(self) -> T;
    unsafe fn write(self, value: T);
    fn cast_const(self) -> *const T;
    fn addr(self) -> usize;
}

impl<T> MutPtr<T> for *mut T {
    fn is_null(self) -> bool {
        self as usize == 0usize
    }
    unsafe fn add(self, count: usize) -> *mut T {
        step::<T>(self as usize, count) as *mut T
    }
    unsafe fn sub(self, count: usize) -> *mut T {
        back::<T>(self as usize, count) as *mut T
    }
    unsafe fn offset(self, count: isize) -> *mut T {
        signed::<T>(self as usize, count) as *mut T
    }
    fn wrapping_add(self, count: usize) -> *mut T {
        wrapping_signed::<T>(self as usize, count as isize) as *mut T
    }
    fn wrapping_sub(self, count: usize) -> *mut T {
        wrapping_signed::<T>(self as usize, 0isize.wrapping_sub(count as isize)) as *mut T
    }
    fn wrapping_offset(self, count: isize) -> *mut T {
        wrapping_signed::<T>(self as usize, count) as *mut T
    }
    unsafe fn offset_from(self, origin: *const T) -> isize {
        ((self as usize).wrapping_sub(origin as usize) as isize) / (super::mem::pointer_stride::<T>() as isize)
    }
    unsafe fn read(self) -> T {
        unsafe { *self }
    }
    unsafe fn write(self, value: T) {
        unsafe {
            *self = value;
        }
    }
    fn cast_const(self) -> *const T {
        self as *const T
    }
    fn addr(self) -> usize {
        self as usize
    }
}
