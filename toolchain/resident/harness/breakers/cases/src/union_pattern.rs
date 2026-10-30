//! tests/ui/union/union-pat-refutability.rs: a union is matched by one of its fields, read
//! where the union is. ra-jit had no lowering for the pattern and left the function to rustc.

#[derive(Clone, Copy)]
#[repr(u32)]
enum Tag {
    Int,
    Pair,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Tagged<T: Copy> {
    tag: Tag,
    value: T,
}

#[repr(C)]
union Value {
    tag: Tag,
    int: Tagged<i64>,
    pair: Tagged<(u16, u16)>,
}

#[inline(never)]
fn read(value: &Value) -> i64 {
    unsafe {
        match *value {
            Value { int: Tagged { tag: Tag::Int, value } } => value,
            Value { pair: Tagged { value: (low, high), .. } } => i64::from(low) + i64::from(high),
        }
    }
}

#[inline(never)]
fn low_byte(value: Value) -> u16 {
    unsafe {
        let Value { pair: Tagged { value: (ref low, _), .. } } = value;
        *low
    }
}

pub fn run() -> (i64, i64, u16) {
    let int = Value { int: Tagged { tag: Tag::Int, value: -7 } };
    let pair = Value { pair: Tagged { tag: Tag::Pair, value: (3, 40) } };
    (read(&int), read(&pair), low_byte(pair))
}
