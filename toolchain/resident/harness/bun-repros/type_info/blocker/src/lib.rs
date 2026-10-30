#![feature(type_info, const_trait_impl, const_cmp, adt_const_params, unsized_const_params)]
#![allow(incomplete_features)]

use core::marker::PhantomData;
use core::mem::type_info;

struct Point<'a> {
    x: u32,
    y: u8,
    name: &'a str,
}

enum Shape {
    Dot,
    Line(u16, u64),
}

struct Reflected<T>(PhantomData<T>);

// What bun's `MultiArrayList` asks of a type: its fields, their sizes, names and offsets,
// and whether a field is of a given type.
impl<T> Reflected<T> {
    const VARIANTS: usize = type_info::of::<T>().variants();
    const FIELDS: usize = type_info::of::<T>().fields(0);
    const SECOND_SIZE: usize = match type_info::of::<T>().field(0, 1).type_id().size() {
        Some(size) => size,
        None => 0,
    };
    const SECOND_NAME: &'static str = type_info::of::<T>().field(0, 1).name();
    const SECOND_OFFSET: usize = type_info::of::<T>().field(0, 1).offset();
    const FIRST_IS_U32: bool = type_info::of::<T>().field(0, 0).type_id() == type_info::of::<u32>();
    const THIRD_IS_STR: bool =
        type_info::of::<T>().field(0, 2).type_id() == type_info::of::<&str>();
}

// What else the pinned nightly's reflection answers: a variant's name, `#[non_exhaustive]`,
// the kind of a type, its generic args and a function pointer's signature.
const VARIANT_NAME: &str = type_info::of::<Shape>().variant(1).name();
const SHAPE_NON_EXHAUSTIVE: bool = type_info::of::<Shape>().non_exhaustive();
const POINT_IS_STRUCT: bool =
    matches!(type_info::Type::of::<Point<'static>>().kind, type_info::TypeKind::Struct);
const POINT_GENERICS: usize = type_info::of::<Point<'static>>().generics().len();
const FN_INPUTS: usize = match type_info::of::<fn(u8, u16) -> u32>().function_ptr() {
    Some(f) => f.inputs().len(),
    None => 0,
};

const fn const_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

impl<T> Reflected<T> {
    // bun's `MultiArrayList::check`: a field found by its name in a `const` block of a
    // generic function called at run time, which only compile time may run.
    const fn index_of<const NAME: &'static str, F>() -> usize {
        const {
            let id = type_info::of::<T>();
            let mut i = 0;
            while i < Self::FIELDS {
                let f = id.field(0, i);
                if const_str_eq(f.name(), NAME) {
                    assert!(f.type_id() == type_info::of::<F>(), "not of that type");
                    break;
                }
                i += 1;
            }
            assert!(i < Self::FIELDS, "no such field");
            i
        }
    }
}

pub fn named(point: &Point<'_>) -> (usize, usize, u32) {
    let x = Reflected::<Point>::index_of::<"x", u32>();
    let name = Reflected::<Point>::index_of::<"name", &str>();
    (x, name, point.x + point.y as u32 + point.name.len() as u32)
}

pub fn fields() -> String {
    format!(
        "{} {} {} {} {} {} {} | {} {} {} | {:?} | {} {} {} {} {}",
        Reflected::<Point>::VARIANTS,
        Reflected::<Point>::FIELDS,
        Reflected::<Point>::SECOND_SIZE,
        Reflected::<Point>::SECOND_NAME,
        Reflected::<Point>::SECOND_OFFSET,
        Reflected::<Point>::FIRST_IS_U32,
        Reflected::<Point>::THIRD_IS_STR,
        Reflected::<Shape>::VARIANTS,
        Reflected::<Shape>::FIELDS,
        Reflected::<(u8, u32)>::SECOND_SIZE,
        named(&Point { x: 4, y: 5, name: "six" }),
        VARIANT_NAME,
        SHAPE_NON_EXHAUSTIVE,
        POINT_IS_STRUCT,
        POINT_GENERICS,
        FN_INPUTS,
    )
}
