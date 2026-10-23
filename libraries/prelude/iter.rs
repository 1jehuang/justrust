//! Native iterator protocols and ordinary source adapters.
use super::option::Option;

pub trait Iterator: Sized {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;

    fn enumerate(self) -> Enumerate<Self> {
        Enumerate { iterator: self, count: 0usize }
    }

    fn copied(self) -> Copied<Self> {
        Copied { iterator: self }
    }

    fn cloned(self) -> Cloned<Self> {
        Cloned { iterator: self }
    }

    fn map<B, F: FnMut(Self::Item) -> B>(self, f: F) -> Map<Self, F> {
        Map { iterator: self, f }
    }

    fn filter<P: FnMut(&Self::Item) -> bool>(self, predicate: P) -> Filter<Self, P> {
        Filter { iterator: self, predicate }
    }

    fn any<F: FnMut(Self::Item) -> bool>(&mut self, mut f: F) -> bool {
        loop {
            match self.next() {
                Option::Some(item) => {
                    if f(item) {
                        return true;
                    }
                }
                Option::None => return false,
            }
        }
    }

    fn all<F: FnMut(Self::Item) -> bool>(&mut self, mut f: F) -> bool {
        loop {
            match self.next() {
                Option::Some(item) => {
                    if !f(item) {
                        return false;
                    }
                }
                Option::None => return true,
            }
        }
    }

    fn position<P: FnMut(Self::Item) -> bool>(&mut self, mut predicate: P) -> Option<usize> {
        let mut index = 0usize;
        loop {
            match self.next() {
                Option::Some(item) => {
                    if predicate(item) {
                        return Option::Some(index);
                    }
                    index += 1usize;
                }
                Option::None => return Option::None,
            }
        }
    }

    fn count(self) -> usize {
        let mut iterator = self;
        let mut count = 0usize;
        loop {
            match iterator.next() {
                Option::Some(_) => count += 1usize,
                Option::None => return count,
            }
        }
    }

    fn collect<B: FromIterator<Self::Item>>(self) -> B
    where
        CollectInput<Self>: IntoIterator<Item = <Self as Iterator>::Item>,
    {
        B::from_iter(CollectInput { iterator: self })
    }

    fn sum<S: Sum<Self::Item>>(self) -> S {
        <S as Sum<Self::Item>>::sum(self)
    }

    fn fold<B, F: FnMut(B, Self::Item) -> B>(mut self, init: B, mut f: F) -> B {
        let mut acc = init;
        loop {
            match self.next() {
                Option::Some(item) => acc = f(acc, item),
                Option::None => return acc,
            }
        }
    }
}

pub trait Sum<A = Self>: Sized {
    fn sum<I: Iterator<Item = A>>(iter: I) -> Self;
}

impl Sum for i8 {
    fn sum<I: Iterator<Item = i8>>(mut iter: I) -> i8 {
        let mut total = 0i8;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a i8> for i8 {
    fn sum<I: Iterator<Item = &'a i8>>(mut iter: I) -> i8 {
        let mut total = 0i8;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for i16 {
    fn sum<I: Iterator<Item = i16>>(mut iter: I) -> i16 {
        let mut total = 0i16;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a i16> for i16 {
    fn sum<I: Iterator<Item = &'a i16>>(mut iter: I) -> i16 {
        let mut total = 0i16;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for i32 {
    fn sum<I: Iterator<Item = i32>>(mut iter: I) -> i32 {
        let mut total = 0i32;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a i32> for i32 {
    fn sum<I: Iterator<Item = &'a i32>>(mut iter: I) -> i32 {
        let mut total = 0i32;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for i64 {
    fn sum<I: Iterator<Item = i64>>(mut iter: I) -> i64 {
        let mut total = 0i64;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a i64> for i64 {
    fn sum<I: Iterator<Item = &'a i64>>(mut iter: I) -> i64 {
        let mut total = 0i64;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for i128 {
    fn sum<I: Iterator<Item = i128>>(mut iter: I) -> i128 {
        let mut total = 0i128;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a i128> for i128 {
    fn sum<I: Iterator<Item = &'a i128>>(mut iter: I) -> i128 {
        let mut total = 0i128;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for isize {
    fn sum<I: Iterator<Item = isize>>(mut iter: I) -> isize {
        let mut total = 0isize;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a isize> for isize {
    fn sum<I: Iterator<Item = &'a isize>>(mut iter: I) -> isize {
        let mut total = 0isize;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for u8 {
    fn sum<I: Iterator<Item = u8>>(mut iter: I) -> u8 {
        let mut total = 0u8;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a u8> for u8 {
    fn sum<I: Iterator<Item = &'a u8>>(mut iter: I) -> u8 {
        let mut total = 0u8;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for u16 {
    fn sum<I: Iterator<Item = u16>>(mut iter: I) -> u16 {
        let mut total = 0u16;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a u16> for u16 {
    fn sum<I: Iterator<Item = &'a u16>>(mut iter: I) -> u16 {
        let mut total = 0u16;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for u32 {
    fn sum<I: Iterator<Item = u32>>(mut iter: I) -> u32 {
        let mut total = 0u32;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a u32> for u32 {
    fn sum<I: Iterator<Item = &'a u32>>(mut iter: I) -> u32 {
        let mut total = 0u32;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for u64 {
    fn sum<I: Iterator<Item = u64>>(mut iter: I) -> u64 {
        let mut total = 0u64;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a u64> for u64 {
    fn sum<I: Iterator<Item = &'a u64>>(mut iter: I) -> u64 {
        let mut total = 0u64;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for u128 {
    fn sum<I: Iterator<Item = u128>>(mut iter: I) -> u128 {
        let mut total = 0u128;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a u128> for u128 {
    fn sum<I: Iterator<Item = &'a u128>>(mut iter: I) -> u128 {
        let mut total = 0u128;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for usize {
    fn sum<I: Iterator<Item = usize>>(mut iter: I) -> usize {
        let mut total = 0usize;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a usize> for usize {
    fn sum<I: Iterator<Item = &'a usize>>(mut iter: I) -> usize {
        let mut total = 0usize;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

pub trait IntoIterator {
    type Item;
    type IntoIter: Iterator<Item = Self::Item>;
    fn into_iter(self) -> Self::IntoIter;
}

pub trait FromIterator<A>: Sized {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self;
}

pub struct CollectInput<I> {
    iterator: I,
}

impl<I: Iterator> IntoIterator for CollectInput<I> {
    type Item = I::Item;
    type IntoIter = I;
    fn into_iter(self) -> I { self.iterator }
}

impl<I: Iterator> IntoIterator for Enumerate<I> {
    type Item = (usize, I::Item);
    type IntoIter = Enumerate<I>;
    fn into_iter(self) -> Enumerate<I> { self }
}

pub struct Enumerate<I> {
    iterator: I,
    count: usize,
}

impl<I: Iterator> Iterator for Enumerate<I> {
    type Item = (usize, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        match self.iterator.next() {
            Option::Some(item) => {
                let index = self.count;
                self.count += 1usize;
                Option::Some((index, item))
            }
            Option::None => Option::None,
        }
    }
}

pub struct Copied<I> {
    iterator: I,
}

impl<'a, T: Copy, I: Iterator<Item = &'a T>> Iterator for Copied<I> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self.iterator.next() {
            Option::Some(item) => Option::Some(*item),
            Option::None => Option::None,
        }
    }
}

pub struct Cloned<I> {
    iterator: I,
}

impl<'a, T: Clone, I: Iterator<Item = &'a T>> Iterator for Cloned<I> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self.iterator.next() {
            Option::Some(item) => Option::Some(item.clone()),
            Option::None => Option::None,
        }
    }
}

pub struct Map<I, F> {
    iterator: I,
    f: F,
}

impl<A, B, I: Iterator<Item = A>, F: FnMut(A) -> B> Iterator for Map<I, F> {
    type Item = B;

    fn next(&mut self) -> Option<B> {
        match self.iterator.next() {
            Option::Some(item) => Option::Some((self.f)(item)),
            Option::None => Option::None,
        }
    }
}

pub struct Filter<I, P> {
    iterator: I,
    predicate: P,
}

impl<A, I: Iterator<Item = A>, P: FnMut(&A) -> bool> Iterator for Filter<I, P> {
    type Item = A;

    fn next(&mut self) -> Option<A> {
        loop {
            match self.iterator.next() {
                Option::Some(item) => {
                    if (self.predicate)(&item) {
                        return Option::Some(item);
                    }
                }
                Option::None => return Option::None,
            }
        }
    }
}
