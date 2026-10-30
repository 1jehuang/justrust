//! async-task keeps `&'static TaskVTable` in a task's header: a borrow of a struct of the
//! addresses of generic functions, written inside a generic function.

use std::marker::PhantomData;

struct Size {
    bytes: usize,
}

struct Table {
    get: unsafe fn(*const ()) -> usize,
    twice: unsafe fn(*const ()) -> usize,
    size: &'static Size,
}

struct Header {
    table: &'static Table,
}

struct Raw<T, S>(PhantomData<(T, S)>);

impl<T: Copy + Into<u64>, S> Raw<T, S> {
    const SIZE: Size = Size { bytes: size_of::<T>() + size_of::<S>() };

    unsafe fn get(at: *const ()) -> usize {
        unsafe { (*(at as *const T)).into() as usize }
    }

    unsafe fn twice(at: *const ()) -> usize {
        unsafe { 2 * Self::get(at) }
    }

    fn header() -> Box<Header> {
        let mut header = Box::<Header>::new_uninit();
        unsafe {
            header.as_mut_ptr().write(Header {
                table: &Table { get: Self::get, twice: Self::twice, size: &Self::SIZE },
            });
            header.assume_init()
        }
    }
}

#[inline(never)]
fn clobber() -> [u64; 64] {
    std::hint::black_box([u64::MAX; 64])
}

pub fn run() -> (usize, usize, usize, usize) {
    let small = Raw::<u8, u16>::header();
    let big = Raw::<u32, ()>::header();
    clobber();
    let (a, b) = (7u8, 9u32);
    unsafe {
        (
            (small.table.get)(&a as *const u8 as *const ()),
            (big.table.twice)(&b as *const u32 as *const ()),
            small.table.size.bytes,
            big.table.size.bytes,
        )
    }
}
