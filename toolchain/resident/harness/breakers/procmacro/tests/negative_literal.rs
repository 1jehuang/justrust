//! `Literal::i8_unsuffixed(-1)` is one token `-1`, which the compiler reads as a minus and a
//! number. ra-jit read it as a number it could not parse, and num_enum's alternative `-1`
//! matched nothing.

#[test]
fn names() {
    assert_eq!([procmacro::name(-1), procmacro::name(0), procmacro::name(1)], ["minus one", "zero", "other"]);
}

#[test]
fn numbers() {
    assert_eq!(procmacro::numbers(), (-1, -20, -1.5));
}
