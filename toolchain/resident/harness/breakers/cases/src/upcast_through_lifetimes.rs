//! tests/ui/traits/trait-upcasting/multiple-supertraits-modulo-binder.rs and
//! regions/regions-early-bound-trait-param.rs: a trait object is coerced to one of a supertrait
//! that the vtable names with other lifetimes, or through an associated type. ra-jit compared
//! the two spellings as written, found no supertrait and refused the function.

trait Get<T> {
    fn get(&self) -> usize;
}

trait Both: Get<fn(&'static u8)> + for<'a> Get<fn(&'a u8, &'a u8)> {}

struct Unit;

impl Get<fn(&'static u8)> for Unit {
    fn get(&self) -> usize {
        1
    }
}

impl<'a> Get<fn(&'a u8, &'a u8)> for Unit {
    fn get(&self) -> usize {
        2
    }
}

impl Both for Unit {}

trait Named {
    type Name;
}

impl Named for () {
    type Name = u16;
}

trait Sized16: Get<<() as Named>::Name> {}

impl Get<u16> for Unit {
    fn get(&self) -> usize {
        16
    }
}

impl Sized16 for Unit {}

trait Borrowed<'a>: Get<&'a str> {}

impl<'a> Get<&'a str> for Unit {
    fn get(&self) -> usize {
        3
    }
}

impl<'a> Borrowed<'a> for Unit {}

#[inline(never)]
fn second(both: &dyn Both) -> usize {
    let get: &dyn for<'a> Get<fn(&'a u8, &'a u8)> = both;
    get.get()
}

#[inline(never)]
fn projected(sized: &dyn Sized16) -> usize {
    let get: &dyn Get<u16> = sized;
    get.get()
}

#[inline(never)]
fn borrowed<'a>(borrowed: Box<dyn Borrowed<'a> + 'a>) -> usize {
    let get: Box<dyn Get<&'a str> + 'a> = borrowed;
    get.get()
}

pub fn run() -> (usize, usize, usize) {
    (second(&Unit), projected(&Unit), borrowed(Box::new(Unit)))
}
