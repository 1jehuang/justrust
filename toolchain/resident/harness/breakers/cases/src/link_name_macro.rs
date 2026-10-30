//! A foreign function whose `#[link_name]` is built by macros, as `ring` names its C and
//! assembly routines: `concat!` around a macro of the crate and `stringify!`.

use std::ffi::{CStr, c_char};

macro_rules! prefix {
    () => {
        concat!("str", "l")
    };
}

macro_rules! prefixed {
    ($attr:ident $name:ident { $item:item }) => {
        #[$attr = concat!(prefix!(), stringify!($name))]
        $item
    };
}

unsafe extern "C" {
    prefixed! {
        link_name
        en
        { fn length_of(text: *const c_char) -> usize; }
    }
}

pub fn run() -> Vec<usize> {
    let texts: [&CStr; 3] = [c"", c"one", c"a longer one"];
    texts.iter().map(|text| unsafe { length_of(text.as_ptr()) }).collect()
}
