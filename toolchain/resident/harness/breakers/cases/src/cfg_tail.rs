//! WIP
pub struct Facts { pub size: usize }
pub enum Info { Sized(usize), Slice(Facts) }
pub trait Known { const INFO: Info; }
impl Known for [u8] { const INFO: Info = Info::Slice(Facts { size: 1 }); }
impl Known for [u16] { const INFO: Info = Info::Slice(Facts { size: 2 }); }

macro_rules! const_panic {
    (@non_panic $($_arg:tt)+) => {{
        let panic: [_; 0] = [];
        panic[0]
    }};
    ($($arg:tt)+) => {{
        #[cfg(not(no_such_cfg))]
        panic!($($arg)+);
        #[cfg(no_such_cfg)]
        const_panic!(@non_panic $($arg)+)
    }};
}

fn facts<T: ?Sized + Known>() -> Facts {
    trait Has { const FACTS: Facts; }
    impl<T: ?Sized + Known> Has for T {
        const FACTS: Facts = match T::INFO {
            Info::Sized(_) => const_panic!("unreachable"),
            Info::Slice(facts) => facts,
        };
    }
    T::FACTS
}

trait Testable {
    fn passing<F: Fn(String)>(f: F);
}
pub trait Zeroed: Default + std::fmt::Debug {}
impl Zeroed for u8 {}
impl Zeroed for u32 {}

fn cases() -> Vec<String> {
    use std::cell::RefCell;
    trait Testable {
        fn passing<F: Fn(String)>(f: F);
    }
    impl<T: Zeroed> Testable for T {
        fn passing<F: Fn(String)>(f: F) { f(format!("{:?}", T::default())) }
    }
    macro_rules! testable {
        ($ty:ty $(,$tys:ty)* => $($case:expr),*) => {
            impl Testable for $ty {
                testable!(@methods $($case),*);
            }
            testable!($($tys),* => $($case),*);
        };
        (=> $($case:expr),*) => {};
        (@methods $($case:expr),*) => {
            fn passing<F: Fn(String)>(f: F) {
                $(f(format!("{:?}", Self::from($case)));)*
            }
        };
    }
    testable!(bool, Option<bool> => true, false);
    let out = RefCell::new(Vec::new());
    macro_rules! all {
        ($($ty:ty),*) => {
            $(<$ty as Testable>::passing(|it| out.borrow_mut().push(it));)*
        };
    }
    all!(u8, u32, bool, Option<bool>);
    out.into_inner()
}

pub struct Holder<T: ?Sized>(std::marker::PhantomData<T>);
impl<T: ?Sized + Known> Holder<T> {
    pub fn size(&self) -> usize { facts::<T>().size }
}

fn nested() -> usize {
    fn inner<const N: usize>() -> usize {
        #[allow(dead_code)]
        struct Dst<const N: usize> { prefix: [u8; N], rest: [u8] }
        impl<const N: usize> Known for Dst<N> {
            const INFO: Info = Info::Slice(Facts { size: N });
        }
        Holder::<Dst<N>>(std::marker::PhantomData).size()
    }
    inner::<3>() + inner::<4>()
}

pub fn run() -> (usize, usize, Vec<String>, usize) {
    (facts::<[u8]>().size, facts::<[u16]>().size, cases(), nested())
}
