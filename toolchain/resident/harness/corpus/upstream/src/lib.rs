//! A library rustc compiles, whose generic code with closures in it `cases` instantiates, as
//! a program does with tonic's `Streaming::new` and its `map_frame(|frame| ..)` (blaze #369).
//! A `TypeId` of a type that holds one of these closures has to be rustc's.

use std::any::Any;

/// Not capturing, `Fn`.
pub fn doubled<I: Iterator<Item = u8>>(it: I) -> impl Iterator<Item = u16> {
    it.map(|x| u16::from(x) * 2)
}

/// Captured by move, `Fn`; the second closure of its function.
pub fn offset<I: Iterator<Item = u8>>(it: I, by: u32) -> impl Iterator<Item = u32> {
    let add = move |x: u8| u32::from(x) + by;
    it.map(|x| x ^ 1).map(add)
}

/// `FnMut`: it changes what it captured.
pub fn counted<I: Iterator<Item = u8>>(it: I) -> impl Iterator<Item = (usize, u8)> {
    let mut n = 0;
    it.map(move |x| {
        n += 1;
        (n, x)
    })
}

/// `FnOnce`: it gives away what it captured.
pub fn once_text(text: String) -> impl FnOnce() -> String {
    move || text
}

/// A closure that makes another: the inner one is a child of the outer one.
pub fn adder(by: u8) -> impl Fn(u8) -> Box<dyn Fn(u8) -> u8> {
    move |first| Box::new(move |second| first + second + by)
}

macro_rules! tripled {
    ($it:expr) => {
        $it.map(|x: u8| u32::from(x) * 3)
    };
}

/// A closure in a macro's expansion is numbered after those written outside of it.
pub fn expanded<I: Iterator<Item = u8>>(it: I) -> (impl Iterator<Item = u32>, impl Fn() -> u8) {
    let mapped = tripled!(it);
    (mapped, || 7)
}

pub struct Wrap<T>(pub T);

impl<T: Clone + 'static> Wrap<T> {
    /// One of an inherent `impl`.
    pub fn repeat(&self) -> impl Fn() -> T {
        let value = self.0.clone();
        move || value.clone()
    }
}

pub trait Source {
    /// One of a provided method of a trait.
    fn source(&self) -> impl Fn() -> u8 {
        || 9
    }
}

impl Source for u8 {}

/// A signature that borrows: `filter` passes `&u8`, which the closure's signature binds.
pub fn kept<I: Iterator<Item = u8>>(it: I) -> impl Iterator<Item = u8> {
    it.filter(|x: &u8| *x > 1)
}

/// A closure in an inline `const` block and one in an array's length are numbered under that
/// anonymous constant, not the function: the returned closure is the function's `{closure#0}`.
pub fn after_consts() -> impl Fn() -> u8 {
    const {
        let _ = || ();
    };
    let _ = [0u8; {
        let _ = || ();
        1
    }];
    || 11
}

/// An `async` block, after a closure of the same function.
pub fn later(x: u8) -> impl std::future::Future<Output = u8> {
    let add = move |y: u8| x + y;
    async move { add(1) }
}

/// The body of an `async fn` is its `{closure#0}`.
pub async fn sum(a: u8, b: u8) -> u8 {
    a + b
}

/// A closure written in an `async fn`: a child of the body's `{closure#0}`.
pub async fn maker(by: u8) -> impl Fn(u8) -> u8 {
    move |x| x * by
}

/// `async` blocks one in another.
pub fn nested(x: u8) -> impl std::future::Future<Output = u8> {
    async move { async move { x }.await + 1 }
}

/// The `try_downcast` of axum-core and tonic: a value goes into `dyn Any` and comes out as
/// `Option<T>` only when it is one.
pub fn try_downcast<T: 'static, K: 'static>(k: K) -> Result<T, K> {
    let mut k = Some(k);
    if let Some(k) = <dyn Any>::downcast_mut::<Option<T>>(&mut k) {
        Ok(k.take().unwrap())
    } else {
        Err(k.unwrap())
    }
}

/// A value of a closure's type boxed by rustc's code, for `cases` to downcast.
pub fn boxed_doubled() -> (impl Iterator<Item = u16>, Box<dyn Any>) {
    (doubled(std::iter::once(1)), Box::new(doubled(std::iter::once(2))))
}

pub enum Shape<T> {
    Dot,
    Line(T),
    Pair(T, T),
}

pub struct Meters(pub f32);

/// A stream mapped through `Ok` boxed by rustc's code, as tonic's client builds its requests.
pub fn boxed_ok() -> (impl Iterator<Item = Result<u8, ()>>, Box<dyn Any>) {
    (std::iter::once(1).map(Ok), Box::new(std::iter::once(2).map(Ok::<u8, ()>)))
}
