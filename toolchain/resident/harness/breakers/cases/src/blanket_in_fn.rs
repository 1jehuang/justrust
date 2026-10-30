//! A blanket impl written in a function is an impl of its block and of its crate, and the
//! solver was given it once for each: two impls that both hold are an ambiguity, and no impl
//! was chosen. A constant of the trait was then evaluated as the trait declares it, without a
//! value. Where the type is known before the function is compiled another path finds the impl;
//! zerocopy's `trailing_slice_layout::<T>()` is reached through a generic method with a type
//! that a test declares in a nested function (3 functions in zerocopy's tests).

use std::marker::PhantomData;

pub struct Facts {
    pub size: usize,
}

pub enum Info {
    Sized(usize),
    Slice(Facts),
}

pub trait Known {
    const INFO: Info;
}

impl Known for [u16] {
    const INFO: Info = Info::Slice(Facts { size: 2 });
}

fn facts<T: ?Sized + Known>() -> Facts {
    trait Has {
        const FACTS: Facts;
    }
    impl<T: ?Sized + Known> Has for T {
        const FACTS: Facts = match T::INFO {
            Info::Sized(_) => panic!("unreachable"),
            Info::Slice(facts) => facts,
        };
    }
    T::FACTS
}

pub struct Holder<T: ?Sized>(PhantomData<T>);

impl<T: ?Sized + Known> Holder<T> {
    pub fn size(&self) -> usize {
        facts::<T>().size
    }
}

fn nested() -> usize {
    fn inner<const N: usize>() -> usize {
        #[allow(dead_code)]
        struct Dst<const N: usize> {
            prefix: [u8; N],
            rest: [u8],
        }
        impl<const N: usize> Known for Dst<N> {
            const INFO: Info = Info::Slice(Facts { size: N });
        }
        Holder::<Dst<N>>(PhantomData).size()
    }
    inner::<3>() + inner::<4>()
}

pub fn run() -> (usize, usize) {
    (facts::<[u16]>().size, nested())
}
