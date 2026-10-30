//! `TypeId` of a type whose arguments are constants other than a `usize`: gix's errors are
//! `Error<E, 'b', 'i'>`, and anyhow downcasts to them by their `TypeId`.

use std::any::{Any, TypeId};

struct Keyed<const KIND: char, const STRICT: bool, const LEVEL: i8, const WIDTH: u16>;

#[inline(never)]
fn is<T: Any>(value: &dyn Any) -> bool {
    value.type_id() == TypeId::of::<T>()
}

pub fn run() -> Vec<bool> {
    let value: Box<dyn Any> = Box::new(Keyed::<'b', true, -3, 700>);
    vec![
        is::<Keyed<'b', true, -3, 700>>(&*value),
        is::<Keyed<'i', true, -3, 700>>(&*value),
        is::<Keyed<'b', false, -3, 700>>(&*value),
        is::<Keyed<'b', true, 3, 700>>(&*value),
        is::<Keyed<'b', true, -3, 701>>(&*value),
        value.downcast_ref::<Keyed<'b', true, -3, 700>>().is_some(),
    ]
}
