//! serde_yaml's `Owned` has `Deref` alone, to a struct holding a raw pointer, and its users
//! write through `addr_of_mut!((*owned.ptr).field)`: the pointer is read through `deref`,
//! and what it points to is written. There is no `deref_mut` to call.

use std::ptr::addr_of_mut;

struct Pair<'a> {
    left: u32,
    right: &'a str,
}

mod owned {
    use std::marker::PhantomData;
    use std::mem::MaybeUninit;
    use std::ops::Deref;
    use std::ptr::{NonNull, addr_of};

    pub struct Owned<T, Init = T> {
        ptr: NonNull<T>,
        marker: PhantomData<NonNull<Init>>,
    }

    #[repr(transparent)]
    pub struct InitPtr<T> {
        pub ptr: *mut T,
    }

    impl<T> Owned<T> {
        pub fn new_uninit() -> Owned<MaybeUninit<T>, T> {
            let boxed = Box::new(MaybeUninit::<T>::uninit());
            let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(boxed)) };
            Owned { ptr, marker: PhantomData }
        }

        pub unsafe fn assume_init(init: Owned<MaybeUninit<T>, T>) -> Owned<T> {
            Owned { ptr: init.ptr.cast(), marker: PhantomData }
        }
    }

    impl<T, Init> Deref for Owned<T, Init> {
        type Target = InitPtr<Init>;
        fn deref(&self) -> &Self::Target {
            unsafe { &*addr_of!(self.ptr).cast::<InitPtr<Init>>() }
        }
    }
}

use owned::Owned;

struct Holder<'a> {
    pin: Owned<Pair<'a>>,
}

impl<'a> Holder<'a> {
    fn new(right: &'a str) -> Holder<'a> {
        let owned = Owned::<Pair>::new_uninit();
        unsafe {
            addr_of_mut!((*owned.ptr).left).write(1);
            addr_of_mut!((*owned.ptr).right).write(right_of(right));
            Holder { pin: Owned::assume_init(owned) }
        }
    }

    fn bump(&mut self) -> u32 {
        unsafe {
            let left = addr_of_mut!((*self.pin.ptr).left);
            *left += 3;
            *left
        }
    }
}

fn right_of(text: &str) -> &str {
    text.trim()
}

pub fn run() -> (u32, usize) {
    let mut holder = Holder::new(" seven ");
    let left = holder.bump();
    let pair = unsafe { Box::from_raw(holder.pin.ptr) };
    (left, pair.right.len())
}
