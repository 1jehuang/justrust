//! pest_generator's `quote!` hands a lifetime written in a macro to `stringify!`, which gives
//! it without a space after the quote.

macro_rules! text {
    ($lifetime:lifetime) => {
        stringify!($lifetime)
    };
    ($other:tt) => {
        stringify!($other)
    };
}

macro_rules! each {
    ($($token:tt)*) => {
        [$(text!($token)),*]
    };
}

macro_rules! written {
    () => {
        each!(State<'_, 'a>)
    };
}

pub fn run() -> ([&'static str; 6], [&'static str; 5], &'static str) {
    (written!(), each!(&'static 'b: 'c), stringify!(fn get<'a>(it: &'a str) -> Ref<'_>))
}
