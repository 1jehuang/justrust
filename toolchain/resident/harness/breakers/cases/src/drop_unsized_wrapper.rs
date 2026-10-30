//! zeroize drops an `Arc<Zeroizing<[u8]>>` in place: the drop glue of a struct whose last
//! field is a slice, with a `Drop` of its own that needs the length.

use std::sync::Arc;

struct Wipe<T: ?Sized + Clear>(T);

trait Clear {
    fn clear(&mut self);
}

impl Clear for [u8] {
    fn clear(&mut self) {
        for byte in self.iter_mut() {
            *byte = 0;
        }
    }
}

impl<const N: usize> Clear for [u8; N] {
    fn clear(&mut self) {
        self.as_mut_slice().clear();
    }
}

impl<T: ?Sized + Clear> Drop for Wipe<T> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

fn through_arc() -> Vec<u8> {
    let mut arc: Arc<Wipe<[u8]>> = Arc::new(Wipe([1, 2, 3, 4]));
    unsafe { std::ptr::drop_in_place(Arc::get_mut(&mut arc).unwrap()) };
    let wiped = arc.0.to_vec();
    std::mem::forget(arc);
    wiped
}

fn through_box() -> Vec<u8> {
    let boxed: Box<Wipe<[u8]>> = Box::new(Wipe([5, 6, 7]));
    let raw = Box::into_raw(boxed);
    unsafe {
        std::ptr::drop_in_place(raw);
        let wiped = (*raw).0.to_vec();
        std::alloc::dealloc(raw.cast(), std::alloc::Layout::for_value(&*raw));
        wiped
    }
}

pub fn run() -> (Vec<u8>, Vec<u8>) {
    (through_arc(), through_box())
}
