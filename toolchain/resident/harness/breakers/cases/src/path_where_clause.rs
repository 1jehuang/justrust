//! nalgebra's `OVector::from_element(..)` names no dimension. The storage of the vector is
//! `<DefaultAllocator as Allocator<?D, U1>>::Buffer`, and that the allocator implements the
//! trait is part of the type being well formed: the where clause of the caller makes `?D` its
//! own `D`. Nothing asked for that, `?D` could as well be the `Dyn` of another impl with a
//! `from_element`, and the path did not resolve: `OPoint::origin` and `from_slice`.

pub trait Dim { fn n() -> usize; }
pub struct U1; pub struct U2; pub struct Dyn;
impl Dim for U1 { fn n() -> usize { 1 } }
impl Dim for U2 { fn n() -> usize { 2 } }
pub trait Alloc<R, C> { type Buffer<T>; fn make<T: Clone>(n: usize, e: T) -> Self::Buffer<T>; }
pub struct Default;
impl<R: Dim, C: Dim> Alloc<R, C> for Default { type Buffer<T> = Vec<T>; fn make<T: Clone>(n: usize, e: T) -> Vec<T> { vec![e; n] } }
impl<C: Dim> Alloc<Dyn, C> for Default { type Buffer<T> = (usize, Vec<T>); fn make<T: Clone>(n: usize, e: T) -> (usize, Vec<T>) { (n, vec![e; n]) } }
pub struct Matrix<T, R, C, S>(pub S, pub std::marker::PhantomData<(T, R, C)>);
pub type Owned<T, R, C> = <Default as Alloc<R, C>>::Buffer<T>;
pub type OMatrix<T, R, C> = Matrix<T, R, C, Owned<T, R, C>>;
pub type OVector<T, D> = Matrix<T, D, U1, Owned<T, D, U1>>;
impl<T: Clone, R: Dim, C: Dim> OMatrix<T, R, C> where Default: Alloc<R, C> {
    pub fn from_element(e: T) -> Self { Matrix(<Default as Alloc<R, C>>::make(R::n() * C::n(), e), std::marker::PhantomData) }
}
impl<T: Clone, C: Dim> OMatrix<T, Dyn, C> where Default: Alloc<Dyn, C> {
    pub fn from_element(n: usize, e: T) -> Self { Matrix(<Default as Alloc<Dyn, C>>::make(n * C::n(), e), std::marker::PhantomData) }
}
pub struct OPoint<T, D: Dim>(pub OVector<T, D>) where Default: Alloc<D, U1>;
impl<T, D: Dim> From<OVector<T, D>> for OPoint<T, D> where Default: Alloc<D, U1> {
    fn from(v: OVector<T, D>) -> Self { OPoint(v) }
}
impl<T: Clone + std::default::Default, D: Dim> OPoint<T, D> where Default: Alloc<D, U1> {
    pub fn origin() -> Self { Self::from(OVector::from_element(T::default())) }
}

pub fn run() -> (Vec<u8>, Vec<u16>) {
    (OPoint::<u8, U2>::origin().0 .0, OPoint::<u16, U1>::origin().0 .0)
}
