//! heapless keeps a queue's buffer in a field typed by an associated type of its storage
//! parameter: an array for the owned storage, a slice for the view. Unsizing the queue to its
//! view reads the length off the field, which is an array only once the projection is normalized.

trait Storage {
    type Buffer<T>: ?Sized + AsRef<[T]>;
}

struct Owned<const N: usize>;
struct View;

impl<const N: usize> Storage for Owned<N> {
    type Buffer<T> = [T; N];
}
impl Storage for View {
    type Buffer<T> = [T];
}

struct Queue<T, S: Storage + ?Sized> {
    head: usize,
    buffer: S::Buffer<T>,
}

impl<T: Copy + Into<u64>> Queue<T, View> {
    fn sum(&self) -> (usize, usize, u64) {
        let buffer = self.buffer.as_ref();
        (self.head, buffer.len(), buffer.iter().map(|&it| it.into()).sum())
    }
}

fn view<T, const N: usize>(queue: &Queue<T, Owned<N>>) -> &Queue<T, View> {
    queue
}

pub fn run() -> [(usize, usize, u64); 2] {
    let small = Queue::<u8, Owned<4>> { head: 1, buffer: [1, 2, 3, 4] };
    let large = Queue::<u32, Owned<10>> { head: 7, buffer: [1000; 10] };
    [view(&small).sum(), view(&large).sum()]
}
