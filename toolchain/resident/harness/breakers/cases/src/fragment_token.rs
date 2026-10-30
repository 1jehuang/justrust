//! What a macro takes as `$e:expr` is one token tree to the macro it passes it on to, also when
//! it is one token. bitvec's `bits![0, 0]` adds sixty-four `0`s to its arguments and ends on the
//! rule that sees nothing but `0`s: taken for one of those, an argument that is `0` ended it at
//! once, with an array of no elements where one was declared (16 functions in bitvec's tests).
//! A type or a path is one token tree in the same way: `bits![u8, crate::order::Msb0; 1]` takes
//! the order as `$order:path` and hands it to a rule that takes it as `$ord:tt`, which got
//! `crate` alone and matched nothing (4 more functions).

macro_rules! written {
    (0) => {
        "the zero of the rule"
    };
    ($l:literal) => {
        "a literal"
    };
    ($t:tt) => {
        "a token tree"
    };
}

macro_rules! as_tree {
    ($t:tt) => {
        written!($t)
    };
}

macro_rules! pass {
    ($e:expr) => {
        [written!($e), as_tree!($e), written!(0), concat!($e, "th"), stringify!($e)]
    };
}

macro_rules! count {
    (; $(0,)*) => { 0usize };
    (; $head:tt, $($rest:tt,)*) => { 1 + count!(; $($rest,)*) };
    ($($val:expr),*) => { count!(; $($val,)* 0, 0,) };
}

macro_rules! trees {
    ($($t:tt)*) => { 0usize $(+ stringify!($t).len().min(1))* };
}

macro_rules! of_type {
    ($t:ty) => {
        trees!($t)
    };
}

macro_rules! of_path {
    ($p:path) => {
        [trees!($p), of_type!($p)]
    };
}

pub fn run() -> ([&'static str; 5], [&'static str; 5], [usize; 3], [usize; 4]) {
    let [path, path_as_type] = of_path!(core::cell::Cell);
    let trees = [of_type!(core::cell::Cell<u8>), of_type!(u8), path, path_as_type];
    (pass!(0), pass!(7), [count!(0, 0), count!(1, 0), count!()], trees)
}
