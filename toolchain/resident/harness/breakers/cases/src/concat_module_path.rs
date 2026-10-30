//! rusty-fork names the test its child runs with `concat!(module_path!(), "::", ...)`.
//! `module_path!()` is expanded to a placeholder, which the literal `concat!` made had in it.

macro_rules! name {
    ($function:ident) => {
        concat!(module_path!(), "::", stringify!($function))
    };
}

mod inner {
    pub fn run() -> (&'static str, &'static str) {
        (name!(run), module_path!())
    }
}

pub fn run() -> [&'static str; 4] {
    let (through, alone) = inner::run();
    [through, alone, name!(run), concat!("in ", module_path!(), " twice ", module_path!())]
}
