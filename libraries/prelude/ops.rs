// Native operator traits. The type checker lowers overloaded operators on
// non-primitive operands to calls of these traits' methods
// (src/compiler/frontend/operators.rs).
pub trait Add<Rhs = Self> {
    type Output;
    fn add(self, rhs: Rhs) -> Self::Output;
}
pub trait Sub<Rhs = Self> {
    type Output;
    fn sub(self, rhs: Rhs) -> Self::Output;
}
pub trait Mul<Rhs = Self> {
    type Output;
    fn mul(self, rhs: Rhs) -> Self::Output;
}
pub trait Div<Rhs = Self> {
    type Output;
    fn div(self, rhs: Rhs) -> Self::Output;
}
pub trait Rem<Rhs = Self> {
    type Output;
    fn rem(self, rhs: Rhs) -> Self::Output;
}
pub trait BitAnd<Rhs = Self> {
    type Output;
    fn bitand(self, rhs: Rhs) -> Self::Output;
}
pub trait BitOr<Rhs = Self> {
    type Output;
    fn bitor(self, rhs: Rhs) -> Self::Output;
}
pub trait BitXor<Rhs = Self> {
    type Output;
    fn bitxor(self, rhs: Rhs) -> Self::Output;
}
pub trait Shl<Rhs = Self> {
    type Output;
    fn shl(self, rhs: Rhs) -> Self::Output;
}
pub trait Shr<Rhs = Self> {
    type Output;
    fn shr(self, rhs: Rhs) -> Self::Output;
}
pub trait Neg {
    type Output;
    fn neg(self) -> Self::Output;
}
pub trait Not {
    type Output;
    fn not(self) -> Self::Output;
}
pub trait AddAssign<Rhs = Self> {
    fn add_assign(&mut self, rhs: Rhs);
}
pub trait SubAssign<Rhs = Self> {
    fn sub_assign(&mut self, rhs: Rhs);
}
pub trait MulAssign<Rhs = Self> {
    fn mul_assign(&mut self, rhs: Rhs);
}
pub trait DivAssign<Rhs = Self> {
    fn div_assign(&mut self, rhs: Rhs);
}
pub trait RemAssign<Rhs = Self> {
    fn rem_assign(&mut self, rhs: Rhs);
}
pub trait BitAndAssign<Rhs = Self> {
    fn bitand_assign(&mut self, rhs: Rhs);
}
pub trait BitOrAssign<Rhs = Self> {
    fn bitor_assign(&mut self, rhs: Rhs);
}
pub trait BitXorAssign<Rhs = Self> {
    fn bitxor_assign(&mut self, rhs: Rhs);
}
pub trait ShlAssign<Rhs = Self> {
    fn shl_assign(&mut self, rhs: Rhs);
}
pub trait ShrAssign<Rhs = Self> {
    fn shr_assign(&mut self, rhs: Rhs);
}
pub trait Index<Idx> {
    type Output;
    fn index(&self, index: Idx) -> &Self::Output;
}
pub trait Deref {
    type Target;
    fn deref(&self) -> &Self::Target;
}
// Callable traits. `Fn(A, B) -> R` sugar parses to `Fn<(A, B), Output = R>`
// and `f(a, b)` on a callable value lowers to `Fn::call(&f, (a, b))`.
// Each trait carries its own `Output` (no supertrait projection yet).
pub trait FnOnce<Args> {
    type Output;
    fn call_once(self, args: Args) -> Self::Output;
}
pub trait FnMut<Args> {
    type Output;
    fn call_mut(&mut self, args: Args) -> Self::Output;
}
pub trait Fn<Args> {
    type Output;
    fn call(&self, args: Args) -> Self::Output;
}
// Range expressions desugar to these struct literals (frontend/range_expr.rs).
#[derive(Clone, PartialEq, Eq)]
pub struct Range<Idx> {
    pub start: Idx,
    pub end: Idx,
}
#[derive(Clone, PartialEq, Eq)]
pub struct RangeFrom<Idx> {
    pub start: Idx,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RangeTo<Idx> {
    pub end: Idx,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RangeFull;
#[derive(Clone, PartialEq, Eq)]
pub struct RangeInclusive<Idx> {
    start: Idx,
    end: Idx,
}
impl<Idx> RangeInclusive<Idx> {
    pub fn new(start: Idx, end: Idx) -> Self {
        Self { start, end }
    }
    pub fn start(&self) -> &Idx { &self.start }
    pub fn end(&self) -> &Idx { &self.end }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RangeToInclusive<Idx> {
    pub end: Idx,
}
