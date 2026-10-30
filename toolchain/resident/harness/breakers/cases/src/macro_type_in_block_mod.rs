//! libc's `c_enum!` test declares a type alias whose type is a macro call, in a module of a
//! function. The module is in the textual scope of the `macro_rules!` before the function, and
//! only the function's own block was: the alias had no type.

use std::any::TypeId;

macro_rules! c_enum {
    ($(#[repr($repr:ty)])? $vis:vis enum $name:ident { $($field_vis:vis $variant:ident = $value:expr,)+ }) => {
        $vis type $name = c_enum!(@ty $($repr)?);
        $($field_vis const $variant: $name = $value;)+
    };
    (@ty $repr:ty) => { $repr };
    (@ty) => { u32 };
}

pub fn run() -> (bool, bool, u8, u16, u16) {
    mod first {
        c_enum! {
            #[repr(u8)]
            pub enum e1 {
                SHARED = 10,
                pub DOUBLE = SHARED * 2,
            }
        }
    }
    mod second {
        c_enum! {
            #[repr(u16)]
            pub enum e2 {
                pub SHARED = 42,
                pub TWICE = SHARED * 2,
            }
        }
    }
    use first::*;
    use second::*;
    (TypeId::of::<e1>() == TypeId::of::<u8>(), TypeId::of::<e2>() == TypeId::of::<u16>(), DOUBLE, SHARED, TWICE)
}
