//! proptest-derive puts its impl in `const _IMPL_ARBITRARY_FOR_X: () = { ... };`. Impls were
//! collected from the blocks of unnamed consts alone, so this one was not an impl of the crate
//! and `Self::Parameters` stayed unknown: im's `Action` and `Operation` strategies, 6 functions.

pub trait Shaped {
    type Parameters;
    fn with(parameters: Self::Parameters) -> Self;
}

#[derive(Debug)]
pub enum Action<A> {
    Insert(A),
    Remove(A),
}

const _IMPL_SHAPED_FOR_ACTION: () = {
    impl<A: Default> Shaped for Action<A> {
        type Parameters = (bool, A);
        fn with(parameters: Self::Parameters) -> Self {
            let (insert, value) = parameters;
            if insert { Action::Insert(value) } else { Action::Remove(A::default()) }
        }
    }
};

pub fn run() -> (Action<u8>, Action<String>) {
    (Shaped::with((true, 7)), Shaped::with((false, "gone".to_owned())))
}
