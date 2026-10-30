//! bevy's `InputFocusPlugin::build` adds an observer of `PropagateEntityTrigger<true, ..>`,
//! whose `type_name` names the `bool` argument. Only a `usize` argument was written, and every
//! function that reached it was left to rustc. rustc writes a `char` as its `Debug` does and the
//! least and the greatest value of an integer type by name.

use std::any::type_name;

pub struct Flag<const ON: bool>;
pub struct Letter<const C: char>;
pub struct Small<const N: i8>;
pub struct Wide<const N: i128>;
pub struct Byte<const N: u8>;
pub struct Count<const N: usize>;
pub struct Offset<const N: isize>;

pub fn run() -> Vec<&'static str> {
    vec![
        type_name::<Flag<true>>(),
        type_name::<Flag<false>>(),
        type_name::<Letter<'a'>>(),
        type_name::<Letter<'\n'>>(),
        type_name::<Letter<'\''>>(),
        type_name::<Letter<'é'>>(),
        type_name::<Small<-1>>(),
        type_name::<Small<{ i8::MIN }>>(),
        type_name::<Small<{ i8::MAX }>>(),
        type_name::<Wide<{ i128::MIN }>>(),
        type_name::<Byte<{ u8::MAX }>>(),
        type_name::<Byte<7>>(),
        type_name::<Count<{ usize::MAX }>>(),
        type_name::<Count<3>>(),
        type_name::<Offset<-2>>(),
        type_name::<Option<Flag<true>>>(),
    ]
}
