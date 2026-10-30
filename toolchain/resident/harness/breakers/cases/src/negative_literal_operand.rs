//! library/std tests/num.rs, `test_pow`: a negative literal a macro took as an expression or
//! as a literal is one operand where the macro puts it, so a method called on it is called on
//! the negative number. ra-jit left it as a sign and a number, and the method took the number.

macro_rules! squared {
    ($num:expr) => {
        $num.pow(2)
    };
}

macro_rules! literal_squared {
    ($num:literal) => {
        $num.pow(2)
    };
}

macro_rules! in_range {
    ($low:expr, $value:expr) => {
        matches!($value, $low..=4)
    };
}

struct Sized<const N: i32>;

impl<const N: i32> Sized<N> {
    fn count(&self) -> i32 {
        N
    }
}

macro_rules! sized {
    ($count:expr) => {
        Sized::<$count>.count()
    };
}

#[inline(never)]
fn operands() -> (i32, i32, i32, bool, i32) {
    (squared!(-4i32), literal_squared!(-3i32), squared!(-2i32) - 1, in_range!(-4, -3), sized!(-7))
}

pub fn run() -> (i32, i32, i32, bool, i32) {
    operands()
}
