// Native comparison traits. Operators on non-primitive operands lower to
// these methods; #[derive(PartialEq, Eq, PartialOrd, Ord)] generates impls.
use super::option::Option;

pub enum Ordering {
    Less,
    Equal,
    Greater,
}

impl Ordering {
    pub fn is_eq(self) -> bool {
        match self {
            Ordering::Equal => true,
            _ => false,
        }
    }
    pub fn is_ne(self) -> bool {
        !self.is_eq()
    }
    pub fn is_lt(self) -> bool {
        match self {
            Ordering::Less => true,
            _ => false,
        }
    }
    pub fn is_gt(self) -> bool {
        match self {
            Ordering::Greater => true,
            _ => false,
        }
    }
    pub fn is_le(self) -> bool {
        !self.is_gt()
    }
    pub fn is_ge(self) -> bool {
        !self.is_lt()
    }
    pub fn reverse(self) -> Ordering {
        match self {
            Ordering::Less => Ordering::Greater,
            Ordering::Equal => Ordering::Equal,
            Ordering::Greater => Ordering::Less,
        }
    }
    pub fn then(self, other: Ordering) -> Ordering {
        match self {
            Ordering::Equal => other,
            o => o,
        }
    }
}

impl Clone for Ordering {
    fn clone(&self) -> Ordering {
        *self
    }
}
impl Copy for Ordering {}

pub trait PartialEq {
    fn eq(&self, other: &Self) -> bool;
    fn ne(&self, other: &Self) -> bool {
        !self.eq(other)
    }
}

pub trait Eq: PartialEq {}

pub trait PartialOrd: PartialEq {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering>;
    fn lt(&self, other: &Self) -> bool {
        match self.partial_cmp(other) {
            Option::Some(Ordering::Less) => true,
            _ => false,
        }
    }
    fn le(&self, other: &Self) -> bool {
        match self.partial_cmp(other) {
            Option::Some(Ordering::Less) => true,
            Option::Some(Ordering::Equal) => true,
            _ => false,
        }
    }
    fn gt(&self, other: &Self) -> bool {
        match self.partial_cmp(other) {
            Option::Some(Ordering::Greater) => true,
            _ => false,
        }
    }
    fn ge(&self, other: &Self) -> bool {
        match self.partial_cmp(other) {
            Option::Some(Ordering::Greater) => true,
            Option::Some(Ordering::Equal) => true,
            _ => false,
        }
    }
}

// TODO(Sized defaults): max/min default methods.
pub trait Ord: Eq + PartialOrd {
    fn cmp(&self, other: &Self) -> Ordering;
}

impl PartialEq for Ordering {
    fn eq(&self, other: &Ordering) -> bool {
        match (*self, *other) {
            (Ordering::Less, Ordering::Less) => true,
            (Ordering::Equal, Ordering::Equal) => true,
            (Ordering::Greater, Ordering::Greater) => true,
            _ => false,
        }
    }
}
impl Eq for Ordering {}

impl PartialEq for i8 {
    fn eq(&self, other: &i8) -> bool {
        *self == *other
    }
}
impl Eq for i8 {}
impl PartialOrd for i8 {
    fn partial_cmp(&self, other: &i8) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for i8 {
    fn cmp(&self, other: &i8) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for i16 {
    fn eq(&self, other: &i16) -> bool {
        *self == *other
    }
}
impl Eq for i16 {}
impl PartialOrd for i16 {
    fn partial_cmp(&self, other: &i16) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for i16 {
    fn cmp(&self, other: &i16) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for i32 {
    fn eq(&self, other: &i32) -> bool {
        *self == *other
    }
}
impl Eq for i32 {}
impl PartialOrd for i32 {
    fn partial_cmp(&self, other: &i32) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for i32 {
    fn cmp(&self, other: &i32) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for i64 {
    fn eq(&self, other: &i64) -> bool {
        *self == *other
    }
}
impl Eq for i64 {}
impl PartialOrd for i64 {
    fn partial_cmp(&self, other: &i64) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for i64 {
    fn cmp(&self, other: &i64) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for i128 {
    fn eq(&self, other: &i128) -> bool {
        *self == *other
    }
}
impl Eq for i128 {}
impl PartialOrd for i128 {
    fn partial_cmp(&self, other: &i128) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for i128 {
    fn cmp(&self, other: &i128) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for isize {
    fn eq(&self, other: &isize) -> bool {
        *self == *other
    }
}
impl Eq for isize {}
impl PartialOrd for isize {
    fn partial_cmp(&self, other: &isize) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for isize {
    fn cmp(&self, other: &isize) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for u8 {
    fn eq(&self, other: &u8) -> bool {
        *self == *other
    }
}
impl Eq for u8 {}
impl PartialOrd for u8 {
    fn partial_cmp(&self, other: &u8) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for u8 {
    fn cmp(&self, other: &u8) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for u16 {
    fn eq(&self, other: &u16) -> bool {
        *self == *other
    }
}
impl Eq for u16 {}
impl PartialOrd for u16 {
    fn partial_cmp(&self, other: &u16) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for u16 {
    fn cmp(&self, other: &u16) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for u32 {
    fn eq(&self, other: &u32) -> bool {
        *self == *other
    }
}
impl Eq for u32 {}
impl PartialOrd for u32 {
    fn partial_cmp(&self, other: &u32) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for u32 {
    fn cmp(&self, other: &u32) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for u64 {
    fn eq(&self, other: &u64) -> bool {
        *self == *other
    }
}
impl Eq for u64 {}
impl PartialOrd for u64 {
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for u64 {
    fn cmp(&self, other: &u64) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for u128 {
    fn eq(&self, other: &u128) -> bool {
        *self == *other
    }
}
impl Eq for u128 {}
impl PartialOrd for u128 {
    fn partial_cmp(&self, other: &u128) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for u128 {
    fn cmp(&self, other: &u128) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for usize {
    fn eq(&self, other: &usize) -> bool {
        *self == *other
    }
}
impl Eq for usize {}
impl PartialOrd for usize {
    fn partial_cmp(&self, other: &usize) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for usize {
    fn cmp(&self, other: &usize) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for char {
    fn eq(&self, other: &char) -> bool {
        *self == *other
    }
}
impl Eq for char {}
impl PartialOrd for char {
    fn partial_cmp(&self, other: &char) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for char {
    fn cmp(&self, other: &char) -> Ordering {
        if *self < *other {
            Ordering::Less
        } else if *self == *other {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl PartialEq for bool {
    fn eq(&self, other: &bool) -> bool {
        *self == *other
    }
}
impl Eq for bool {}
impl PartialOrd for bool {
    fn partial_cmp(&self, other: &bool) -> Option<Ordering> {
        Option::Some(self.cmp(other))
    }
}
impl Ord for bool {
    fn cmp(&self, other: &bool) -> Ordering {
        match (*self, *other) {
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}


impl<A: PartialEq> PartialEq for (A,) {
    fn eq(&self, other: &(A,)) -> bool {
        <A as PartialEq>::eq(&(*self).0, &(*other).0)
    }
}
impl<A: Eq> Eq for (A,) {}
impl<A: PartialOrd> PartialOrd for (A,) {
    fn partial_cmp(&self, other: &(A,)) -> Option<Ordering> {
        match <A as PartialOrd>::partial_cmp(&(*self).0, &(*other).0) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        Option::Some(Ordering::Equal)
    }
}
impl<A: Ord> Ord for (A,) {
    fn cmp(&self, other: &(A,)) -> Ordering {
        match <A as Ord>::cmp(&(*self).0, &(*other).0) {
            Ordering::Equal => {}
            c => return c,
        }
        Ordering::Equal
    }
}

impl<A: PartialEq, B: PartialEq> PartialEq for (A, B) {
    fn eq(&self, other: &(A, B)) -> bool {
        <A as PartialEq>::eq(&(*self).0, &(*other).0) && <B as PartialEq>::eq(&(*self).1, &(*other).1)
    }
}
impl<A: Eq, B: Eq> Eq for (A, B) {}
impl<A: PartialOrd, B: PartialOrd> PartialOrd for (A, B) {
    fn partial_cmp(&self, other: &(A, B)) -> Option<Ordering> {
        match <A as PartialOrd>::partial_cmp(&(*self).0, &(*other).0) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        match <B as PartialOrd>::partial_cmp(&(*self).1, &(*other).1) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        Option::Some(Ordering::Equal)
    }
}
impl<A: Ord, B: Ord> Ord for (A, B) {
    fn cmp(&self, other: &(A, B)) -> Ordering {
        match <A as Ord>::cmp(&(*self).0, &(*other).0) {
            Ordering::Equal => {}
            c => return c,
        }
        match <B as Ord>::cmp(&(*self).1, &(*other).1) {
            Ordering::Equal => {}
            c => return c,
        }
        Ordering::Equal
    }
}

impl<A: PartialEq, B: PartialEq, C: PartialEq> PartialEq for (A, B, C) {
    fn eq(&self, other: &(A, B, C)) -> bool {
        <A as PartialEq>::eq(&(*self).0, &(*other).0) && <B as PartialEq>::eq(&(*self).1, &(*other).1) && <C as PartialEq>::eq(&(*self).2, &(*other).2)
    }
}
impl<A: Eq, B: Eq, C: Eq> Eq for (A, B, C) {}
impl<A: PartialOrd, B: PartialOrd, C: PartialOrd> PartialOrd for (A, B, C) {
    fn partial_cmp(&self, other: &(A, B, C)) -> Option<Ordering> {
        match <A as PartialOrd>::partial_cmp(&(*self).0, &(*other).0) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        match <B as PartialOrd>::partial_cmp(&(*self).1, &(*other).1) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        match <C as PartialOrd>::partial_cmp(&(*self).2, &(*other).2) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        Option::Some(Ordering::Equal)
    }
}
impl<A: Ord, B: Ord, C: Ord> Ord for (A, B, C) {
    fn cmp(&self, other: &(A, B, C)) -> Ordering {
        match <A as Ord>::cmp(&(*self).0, &(*other).0) {
            Ordering::Equal => {}
            c => return c,
        }
        match <B as Ord>::cmp(&(*self).1, &(*other).1) {
            Ordering::Equal => {}
            c => return c,
        }
        match <C as Ord>::cmp(&(*self).2, &(*other).2) {
            Ordering::Equal => {}
            c => return c,
        }
        Ordering::Equal
    }
}

impl<A: PartialEq, B: PartialEq, C: PartialEq, D: PartialEq> PartialEq for (A, B, C, D) {
    fn eq(&self, other: &(A, B, C, D)) -> bool {
        <A as PartialEq>::eq(&(*self).0, &(*other).0) && <B as PartialEq>::eq(&(*self).1, &(*other).1) && <C as PartialEq>::eq(&(*self).2, &(*other).2) && <D as PartialEq>::eq(&(*self).3, &(*other).3)
    }
}
impl<A: Eq, B: Eq, C: Eq, D: Eq> Eq for (A, B, C, D) {}
impl<A: PartialOrd, B: PartialOrd, C: PartialOrd, D: PartialOrd> PartialOrd for (A, B, C, D) {
    fn partial_cmp(&self, other: &(A, B, C, D)) -> Option<Ordering> {
        match <A as PartialOrd>::partial_cmp(&(*self).0, &(*other).0) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        match <B as PartialOrd>::partial_cmp(&(*self).1, &(*other).1) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        match <C as PartialOrd>::partial_cmp(&(*self).2, &(*other).2) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        match <D as PartialOrd>::partial_cmp(&(*self).3, &(*other).3) {
            Option::Some(Ordering::Equal) => {}
            c => return c,
        }
        Option::Some(Ordering::Equal)
    }
}
impl<A: Ord, B: Ord, C: Ord, D: Ord> Ord for (A, B, C, D) {
    fn cmp(&self, other: &(A, B, C, D)) -> Ordering {
        match <A as Ord>::cmp(&(*self).0, &(*other).0) {
            Ordering::Equal => {}
            c => return c,
        }
        match <B as Ord>::cmp(&(*self).1, &(*other).1) {
            Ordering::Equal => {}
            c => return c,
        }
        match <C as Ord>::cmp(&(*self).2, &(*other).2) {
            Ordering::Equal => {}
            c => return c,
        }
        match <D as Ord>::cmp(&(*self).3, &(*other).3) {
            Ordering::Equal => {}
            c => return c,
        }
        Ordering::Equal
    }
}

/// `std::cmp::max`: the second argument when equal, like std.
pub fn max<T: Ord>(a: T, b: T) -> T {
    match a.cmp(&b) {
        Ordering::Greater => a,
        _ => b,
    }
}

/// `std::cmp::min`: the first argument when equal, like std.
pub fn min<T: Ord>(a: T, b: T) -> T {
    match a.cmp(&b) {
        Ordering::Greater => b,
        _ => a,
    }
}
