// Native Default trait; #[derive(Default)] generates impls.
pub trait Default {
    fn default() -> Self;
}

impl Default for i8 {
    fn default() -> i8 {
        0
    }
}

impl Default for i16 {
    fn default() -> i16 {
        0
    }
}

impl Default for i32 {
    fn default() -> i32 {
        0
    }
}

impl Default for i64 {
    fn default() -> i64 {
        0
    }
}

impl Default for isize {
    fn default() -> isize {
        0
    }
}

impl Default for u8 {
    fn default() -> u8 {
        0
    }
}

impl Default for u16 {
    fn default() -> u16 {
        0
    }
}

impl Default for u32 {
    fn default() -> u32 {
        0
    }
}

impl Default for u64 {
    fn default() -> u64 {
        0
    }
}

impl Default for usize {
    fn default() -> usize {
        0
    }
}

impl Default for i128 {
    fn default() -> i128 {
        0
    }
}

impl Default for u128 {
    fn default() -> u128 {
        0
    }
}

impl Default for bool {
    fn default() -> bool {
        false
    }
}

impl Default for char {
    fn default() -> char {
        '\x00'
    }
}

impl Default for () {
    fn default() -> () {}
}


impl<A: Default> Default for (A,) {
    fn default() -> (A,) {
        (<A as Default>::default(),)
    }
}

impl<A: Default, B: Default> Default for (A, B) {
    fn default() -> (A, B) {
        (<A as Default>::default(), <B as Default>::default())
    }
}

impl<A: Default, B: Default, C: Default> Default for (A, B, C) {
    fn default() -> (A, B, C) {
        (<A as Default>::default(), <B as Default>::default(), <C as Default>::default())
    }
}

impl<A: Default, B: Default, C: Default, D: Default> Default for (A, B, C, D) {
    fn default() -> (A, B, C, D) {
        (<A as Default>::default(), <B as Default>::default(), <C as Default>::default(), <D as Default>::default())
    }
}
