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
pub trait IndexMut<Idx>: Index<Idx> {
    fn index_mut(&mut self, index: Idx) -> &mut Self::Output;
}
pub trait Deref {
    type Target: ?Sized;
    fn deref(&self) -> &Self::Target;
}
pub trait DerefMut: Deref {
    fn deref_mut(&mut self) -> &mut Self::Target;
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
    exhausted: bool,
}
impl<Idx> RangeInclusive<Idx> {
    pub fn new(start: Idx, end: Idx) -> Self {
        RangeInclusive { start, end, exhausted: false }
    }
    pub fn start(&self) -> &Idx { &self.start }
    pub fn end(&self) -> &Idx { &self.end }
    pub fn into_inner(self) -> (Idx, Idx) { (self.start, self.end) }
    // Internal iterator hooks keep the standard bounds private. The iterator
    // checks equality before advancing at MIN/MAX and marks the final item
    // exhausted without performing an overflowing successor/predecessor.
    pub fn __jr_set_start(&mut self, next: Idx) { self.start = next; }
    pub fn __jr_set_end(&mut self, next: Idx) { self.end = next; }
    pub fn __jr_exhaust(&mut self) { self.exhausted = true; }
}
impl<Idx: super::cmp::PartialOrd> RangeInclusive<Idx> {
    pub fn is_empty(&self) -> bool { self.exhausted || !(self.start <= self.end) }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RangeToInclusive<Idx> {
    pub end: Idx,
}

// Inclusive iterators stop at the last item before attempting a successor.
impl super::iter::Iterator for RangeInclusive<i8> {
    type Item = i8;
    fn next(&mut self) -> super::option::Option<i8> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1i8; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<i8> {
    fn next_back(&mut self) -> super::option::Option<i8> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1i8; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<i8> {
    fn next_back(&mut self) -> super::option::Option<i8> {
        if self.start < self.end {
            self.end -= 1i8;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<i16> {
    type Item = i16;
    fn next(&mut self) -> super::option::Option<i16> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1i16; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<i16> {
    fn next_back(&mut self) -> super::option::Option<i16> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1i16; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<i16> {
    fn next_back(&mut self) -> super::option::Option<i16> {
        if self.start < self.end {
            self.end -= 1i16;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<i32> {
    type Item = i32;
    fn next(&mut self) -> super::option::Option<i32> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1i32; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<i32> {
    fn next_back(&mut self) -> super::option::Option<i32> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1i32; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<i32> {
    fn next_back(&mut self) -> super::option::Option<i32> {
        if self.start < self.end {
            self.end -= 1i32;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<i64> {
    type Item = i64;
    fn next(&mut self) -> super::option::Option<i64> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1i64; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<i64> {
    fn next_back(&mut self) -> super::option::Option<i64> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1i64; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<i64> {
    fn next_back(&mut self) -> super::option::Option<i64> {
        if self.start < self.end {
            self.end -= 1i64;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<i128> {
    type Item = i128;
    fn next(&mut self) -> super::option::Option<i128> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1i128; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<i128> {
    fn next_back(&mut self) -> super::option::Option<i128> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1i128; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<i128> {
    fn next_back(&mut self) -> super::option::Option<i128> {
        if self.start < self.end {
            self.end -= 1i128;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<isize> {
    type Item = isize;
    fn next(&mut self) -> super::option::Option<isize> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1isize; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<isize> {
    fn next_back(&mut self) -> super::option::Option<isize> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1isize; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<isize> {
    fn next_back(&mut self) -> super::option::Option<isize> {
        if self.start < self.end {
            self.end -= 1isize;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<u8> {
    type Item = u8;
    fn next(&mut self) -> super::option::Option<u8> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1u8; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<u8> {
    fn next_back(&mut self) -> super::option::Option<u8> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1u8; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<u8> {
    fn next_back(&mut self) -> super::option::Option<u8> {
        if self.start < self.end {
            self.end -= 1u8;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<u16> {
    type Item = u16;
    fn next(&mut self) -> super::option::Option<u16> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1u16; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<u16> {
    fn next_back(&mut self) -> super::option::Option<u16> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1u16; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<u16> {
    fn next_back(&mut self) -> super::option::Option<u16> {
        if self.start < self.end {
            self.end -= 1u16;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<u32> {
    type Item = u32;
    fn next(&mut self) -> super::option::Option<u32> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1u32; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<u32> {
    fn next_back(&mut self) -> super::option::Option<u32> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1u32; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<u32> {
    fn next_back(&mut self) -> super::option::Option<u32> {
        if self.start < self.end {
            self.end -= 1u32;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<u64> {
    type Item = u64;
    fn next(&mut self) -> super::option::Option<u64> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1u64; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<u64> {
    fn next_back(&mut self) -> super::option::Option<u64> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1u64; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<u64> {
    fn next_back(&mut self) -> super::option::Option<u64> {
        if self.start < self.end {
            self.end -= 1u64;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<u128> {
    type Item = u128;
    fn next(&mut self) -> super::option::Option<u128> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1u128; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<u128> {
    fn next_back(&mut self) -> super::option::Option<u128> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1u128; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<u128> {
    fn next_back(&mut self) -> super::option::Option<u128> {
        if self.start < self.end {
            self.end -= 1u128;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
impl super::iter::Iterator for RangeInclusive<usize> {
    type Item = usize;
    fn next(&mut self) -> super::option::Option<usize> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.start;
        if value == self.end { self.exhausted = true; }
        else { self.start = value + 1usize; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for RangeInclusive<usize> {
    fn next_back(&mut self) -> super::option::Option<usize> {
        if self.is_empty() { return super::option::Option::None; }
        let value = self.end;
        if value == self.start { self.exhausted = true; }
        else { self.end = value - 1usize; }
        super::option::Option::Some(value)
    }
}
impl super::iter::DoubleEndedIterator for Range<usize> {
    fn next_back(&mut self) -> super::option::Option<usize> {
        if self.start < self.end {
            self.end -= 1usize;
            super::option::Option::Some(self.end)
        } else { super::option::Option::None }
    }
}
