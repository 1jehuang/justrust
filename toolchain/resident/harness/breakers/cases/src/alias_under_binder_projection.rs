//! yoke names `for<'a> fn(<Y as Yokeable<'a>>::Output)`: a projection that mentions the
//! lifetime of the binder around it. Normalizing it with the lifetime still bound panicked.

use std::borrow::Cow;

trait Lend<'a> {
    type Out: 'a;
}

impl<'a> Lend<'a> for Cow<'static, str> {
    type Out = Cow<'a, str>;
}

impl<'a> Lend<'a> for u32 {
    type Out = &'a u32;
}

fn attach<Y: for<'a> Lend<'a>, R>(
    cart: &str,
    f: for<'a> fn(&'a str) -> <Y as Lend<'a>>::Out,
    g: for<'a> fn(<Y as Lend<'a>>::Out) -> R,
) -> R {
    g(f(cart))
}

fn borrow(text: &str) -> Cow<'_, str> {
    Cow::Borrowed(text)
}

fn length(text: Cow<'_, str>) -> usize {
    text.len()
}

fn pick(_: &str) -> &u32 {
    &7
}

pub fn run() -> (usize, u32) {
    (
        attach::<Cow<'static, str>, _>("yoked", borrow, length),
        attach::<u32, _>("", pick, |n| *n + 1),
    )
}
