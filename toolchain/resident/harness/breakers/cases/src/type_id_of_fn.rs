//! test-log hands tracing-subscriber `TestWriter::with_stderr`, the function, to write with,
//! and `tracing` asks every layer for its `TypeId`: zbus sets one up in each of its tests.

use std::any::{Any, TypeId};
use std::marker::PhantomData;

struct Layer<W>(PhantomData<W>);

fn layer<W>(_: W) -> Layer<W> {
    Layer(PhantomData)
}

fn same<A: Any, B: Any>(_: &A, b: &B) -> bool {
    TypeId::of::<A>() == b.type_id()
}

/// The hash itself: rustc's code and ra-jit's have to agree on it.
fn id<A: Any>(_: &A) -> TypeId {
    TypeId::of::<A>()
}

struct Writer<T>(T);

impl Writer<u8> {
    fn narrow() -> Self {
        Writer(0)
    }
}

impl<T: Default> Writer<T> {
    fn any() -> Self {
        Writer(T::default())
    }
}

pub fn run() -> ([bool; 4], [TypeId; 3], u8, u16) {
    let (out, narrow, any) = (layer(std::io::stdout), layer(Writer::narrow), layer(Writer::<u16>::any));
    let known = [
        same(&out, &layer(std::io::stdout)),
        same(&narrow, &layer(Writer::narrow)),
        same(&any, &layer(Writer::<u16>::any)),
        same(&any, &layer(Writer::<u32>::any)),
    ];
    (known, [id(&out), id(&narrow), id(&any)], Writer::narrow().0, Writer::<u16>::any().0)
}
