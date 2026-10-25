//! Native iterator protocols and ordinary source adapters.
use super::option::Option;

/// Keeps `best` when `compare(best, item)` is Greater exactly when
/// `keep_on_greater`, else takes `item`. Owning both values avoids a loan of
/// `best` living across the caller's loop backedge.
fn select_by<T, F: FnMut(&T, &T) -> super::cmp::Ordering>(
    best: T,
    item: T,
    keep_on_greater: bool,
    compare: &mut F,
) -> T {
    let greater = compare(&best, &item) == super::cmp::Ordering::Greater;
    if greater != keep_on_greater { best } else { item }
}

pub trait Iterator: Sized {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;

    fn nth(&mut self, mut n: usize) -> Option<Self::Item> {
        loop {
            match self.next() {
                Option::Some(item) => {
                    if n == 0usize {
                        return Option::Some(item);
                    }
                    n -= 1usize;
                }
                Option::None => return Option::None,
            }
        }
    }

    fn last(mut self) -> Option<Self::Item> {
        let mut last = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        loop {
            match self.next() {
                Option::Some(item) => last = item,
                Option::None => return Option::Some(last),
            }
        }
    }

    fn find<P: FnMut(&Self::Item) -> bool>(&mut self, mut predicate: P) -> Option<Self::Item> {
        loop {
            match self.next() {
                Option::Some(item) => {
                    if predicate(&item) {
                        return Option::Some(item);
                    }
                }
                Option::None => return Option::None,
            }
        }
    }

    fn enumerate(self) -> Enumerate<Self> {
        Enumerate { iterator: self, count: 0usize }
    }

    fn rev(self) -> Rev<Self>
    where
        Self: DoubleEndedIterator,
    {
        Rev { iterator: self }
    }

    fn zip<U: IntoIterator>(self, other: U) -> Zip<Self, U::IntoIter> {
        Zip { left: self, right: other.into_iter() }
    }

    fn skip(self, n: usize) -> Skip<Self> {
        Skip { iterator: self, remaining: n }
    }

    fn take(self, n: usize) -> Take<Self> {
        Take { iterator: self, remaining: n }
    }

    fn step_by(self, step: usize) -> StepBy<Self> {
        assert!(step != 0usize);
        StepBy { iterator: self, skip: step - 1usize, first: true }
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

    fn product<P: Product<Self::Item>>(self) -> P {
        <P as Product<Self::Item>>::product(self)
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
    /// The last maximum under `compare`, as std's `max_by`.
    fn max_by<F: FnMut(&Self::Item, &Self::Item) -> super::cmp::Ordering>(
        mut self,
        mut compare: F,
    ) -> Option<Self::Item> {
        let mut best = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        loop {
            match self.next() {
                Option::Some(item) => best = select_by(best, item, false, &mut compare),
                Option::None => return Option::Some(best),
            }
        }
    }
    /// The first minimum under `compare`, as std's `min_by`.
    fn min_by<F: FnMut(&Self::Item, &Self::Item) -> super::cmp::Ordering>(
        mut self,
        mut compare: F,
    ) -> Option<Self::Item> {
        let mut best = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        loop {
            match self.next() {
                Option::Some(item) => best = select_by(best, item, true, &mut compare),
                Option::None => return Option::Some(best),
            }
        }
    }
    /// The last maximum, as std's `max`.
    fn max(mut self) -> Option<Self::Item>
    where
        Self::Item: super::cmp::Ord,
    {
        let mut best = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        loop {
            match self.next() {
                Option::Some(item) => {
                    if item.cmp(&best) != super::cmp::Ordering::Less {
                        best = item;
                    }
                }
                Option::None => return Option::Some(best),
            }
        }
    }
    /// The first minimum, as std's `min`.
    fn min(mut self) -> Option<Self::Item>
    where
        Self::Item: super::cmp::Ord,
    {
        let mut best = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        loop {
            match self.next() {
                Option::Some(item) => {
                    if item.cmp(&best) == super::cmp::Ordering::Less {
                        best = item;
                    }
                }
                Option::None => return Option::Some(best),
            }
        }
    }
    /// The last maximum by `key`, evaluating `key` once per item as std does.
    fn max_by_key<B: super::cmp::Ord, F: FnMut(&Self::Item) -> B>(
        mut self,
        mut key: F,
    ) -> Option<Self::Item> {
        let mut best = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        let mut best_key = key(&best);
        loop {
            match self.next() {
                Option::Some(item) => {
                    let k = key(&item);
                    if k.cmp(&best_key) != super::cmp::Ordering::Less {
                        best = item;
                        best_key = k;
                    }
                }
                Option::None => return Option::Some(best),
            }
        }
    }
    /// The first minimum by `key`, evaluating `key` once per item as std does.
    fn min_by_key<B: super::cmp::Ord, F: FnMut(&Self::Item) -> B>(
        mut self,
        mut key: F,
    ) -> Option<Self::Item> {
        let mut best = match self.next() {
            Option::Some(item) => item,
            Option::None => return Option::None,
        };
        let mut best_key = key(&best);
        loop {
            match self.next() {
                Option::Some(item) => {
                    let k = key(&item);
                    if k.cmp(&best_key) == super::cmp::Ordering::Less {
                        best = item;
                        best_key = k;
                    }
                }
                Option::None => return Option::Some(best),
            }
        }
    }
}

pub trait DoubleEndedIterator: Iterator {
    fn next_back(&mut self) -> Option<Self::Item>;
}

pub struct Rev<I> {
    iterator: I,
}

impl<I: DoubleEndedIterator> Iterator for Rev<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next_back()
    }
}

impl<I: DoubleEndedIterator> DoubleEndedIterator for Rev<I> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iterator.next()
    }
}

impl<I: DoubleEndedIterator> IntoIterator for Rev<I> {
    type Item = I::Item;
    type IntoIter = Self;
    fn into_iter(self) -> Self {
        self
    }
}

impl<T> IntoIterator for super::ops::Range<T>
where
    super::ops::Range<T>: Iterator,
{
    type Item = <super::ops::Range<T> as Iterator>::Item;
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

impl<T> IntoIterator for super::ops::RangeInclusive<T>
where
    super::ops::RangeInclusive<T>: Iterator,
{
    type Item = <super::ops::RangeInclusive<T> as Iterator>::Item;
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

impl<A: Iterator, B: Iterator> IntoIterator for Zip<A, B> {
    type Item = (A::Item, B::Item);
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

impl<I: Iterator> IntoIterator for Skip<I> {
    type Item = I::Item;
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

impl<I: Iterator> IntoIterator for Take<I> {
    type Item = I::Item;
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

impl<I: Iterator> IntoIterator for StepBy<I> {
    type Item = I::Item;
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

impl<I: Iterator> IntoIterator for Enumerate<I> {
    type Item = (usize, I::Item);
    type IntoIter = Enumerate<I>;
    fn into_iter(self) -> Enumerate<I> { self }
}

pub struct Zip<A, B> {
    left: A,
    right: B,
}

impl<A: Iterator, B: Iterator> Iterator for Zip<A, B> {
    type Item = (A::Item, B::Item);
    fn next(&mut self) -> Option<Self::Item> {
        match self.left.next() {
            Option::Some(left) => match self.right.next() {
                Option::Some(right) => Option::Some((left, right)),
                Option::None => Option::None,
            },
            Option::None => Option::None,
        }
    }
}

pub struct Skip<I> {
    iterator: I,
    remaining: usize,
}

impl<I: Iterator> Iterator for Skip<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        let mut remaining = self.remaining;
        self.remaining = 0usize;
        while remaining != 0usize {
            remaining -= 1usize;
            match self.iterator.next() {
                Option::Some(_) => {},
                Option::None => return Option::None,
            }
        }
        self.iterator.next()
    }
}

pub struct Take<I> {
    iterator: I,
    remaining: usize,
}

impl<I: Iterator> Iterator for Take<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0usize {
            return Option::None;
        }
        self.remaining -= 1usize;
        self.iterator.next()
    }
}

pub struct StepBy<I> {
    iterator: I,
    skip: usize,
    first: bool,
}

impl<I: Iterator> Iterator for StepBy<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.first {
            self.first = false;
            return self.iterator.next();
        }
        let mut remaining = self.skip;
        while remaining != 0usize {
            remaining -= 1usize;
            match self.iterator.next() {
                Option::Some(_) => {},
                Option::None => return Option::None,
            }
        }
        self.iterator.next()
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

// Float sums start at -0.0 like stock, so an empty or all -0.0 sum is -0.0.
impl Sum for f64 {
    fn sum<I: Iterator<Item = f64>>(mut iter: I) -> f64 {
        let mut total = -0.0f64;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a f64> for f64 {
    fn sum<I: Iterator<Item = &'a f64>>(mut iter: I) -> f64 {
        let mut total = -0.0f64;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

impl Sum for f32 {
    fn sum<I: Iterator<Item = f32>>(mut iter: I) -> f32 {
        let mut total = -0.0f32;
        loop {
            match iter.next() {
                Option::Some(value) => total += value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Sum<&'a f32> for f32 {
    fn sum<I: Iterator<Item = &'a f32>>(mut iter: I) -> f32 {
        let mut total = -0.0f32;
        loop {
            match iter.next() {
                Option::Some(value) => total += *value,
                Option::None => return total,
            }
        }
    }
}

pub trait Product<A = Self>: Sized {
    fn product<I: Iterator<Item = A>>(iter: I) -> Self;
}

impl Product for i8 {
    fn product<I: Iterator<Item = i8>>(mut iter: I) -> i8 {
        let mut total = 1i8;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a i8> for i8 {
    fn product<I: Iterator<Item = &'a i8>>(mut iter: I) -> i8 {
        let mut total = 1i8;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for i16 {
    fn product<I: Iterator<Item = i16>>(mut iter: I) -> i16 {
        let mut total = 1i16;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a i16> for i16 {
    fn product<I: Iterator<Item = &'a i16>>(mut iter: I) -> i16 {
        let mut total = 1i16;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for i32 {
    fn product<I: Iterator<Item = i32>>(mut iter: I) -> i32 {
        let mut total = 1i32;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a i32> for i32 {
    fn product<I: Iterator<Item = &'a i32>>(mut iter: I) -> i32 {
        let mut total = 1i32;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for i64 {
    fn product<I: Iterator<Item = i64>>(mut iter: I) -> i64 {
        let mut total = 1i64;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a i64> for i64 {
    fn product<I: Iterator<Item = &'a i64>>(mut iter: I) -> i64 {
        let mut total = 1i64;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for i128 {
    fn product<I: Iterator<Item = i128>>(mut iter: I) -> i128 {
        let mut total = 1i128;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a i128> for i128 {
    fn product<I: Iterator<Item = &'a i128>>(mut iter: I) -> i128 {
        let mut total = 1i128;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for isize {
    fn product<I: Iterator<Item = isize>>(mut iter: I) -> isize {
        let mut total = 1isize;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a isize> for isize {
    fn product<I: Iterator<Item = &'a isize>>(mut iter: I) -> isize {
        let mut total = 1isize;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for u8 {
    fn product<I: Iterator<Item = u8>>(mut iter: I) -> u8 {
        let mut total = 1u8;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a u8> for u8 {
    fn product<I: Iterator<Item = &'a u8>>(mut iter: I) -> u8 {
        let mut total = 1u8;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for u16 {
    fn product<I: Iterator<Item = u16>>(mut iter: I) -> u16 {
        let mut total = 1u16;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a u16> for u16 {
    fn product<I: Iterator<Item = &'a u16>>(mut iter: I) -> u16 {
        let mut total = 1u16;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for u32 {
    fn product<I: Iterator<Item = u32>>(mut iter: I) -> u32 {
        let mut total = 1u32;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a u32> for u32 {
    fn product<I: Iterator<Item = &'a u32>>(mut iter: I) -> u32 {
        let mut total = 1u32;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for u64 {
    fn product<I: Iterator<Item = u64>>(mut iter: I) -> u64 {
        let mut total = 1u64;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a u64> for u64 {
    fn product<I: Iterator<Item = &'a u64>>(mut iter: I) -> u64 {
        let mut total = 1u64;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for u128 {
    fn product<I: Iterator<Item = u128>>(mut iter: I) -> u128 {
        let mut total = 1u128;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a u128> for u128 {
    fn product<I: Iterator<Item = &'a u128>>(mut iter: I) -> u128 {
        let mut total = 1u128;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for usize {
    fn product<I: Iterator<Item = usize>>(mut iter: I) -> usize {
        let mut total = 1usize;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a usize> for usize {
    fn product<I: Iterator<Item = &'a usize>>(mut iter: I) -> usize {
        let mut total = 1usize;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for f64 {
    fn product<I: Iterator<Item = f64>>(mut iter: I) -> f64 {
        let mut total = 1.0f64;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a f64> for f64 {
    fn product<I: Iterator<Item = &'a f64>>(mut iter: I) -> f64 {
        let mut total = 1.0f64;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
                Option::None => return total,
            }
        }
    }
}

impl Product for f32 {
    fn product<I: Iterator<Item = f32>>(mut iter: I) -> f32 {
        let mut total = 1.0f32;
        loop {
            match iter.next() {
                Option::Some(value) => total *= value,
                Option::None => return total,
            }
        }
    }
}

impl<'a> Product<&'a f32> for f32 {
    fn product<I: Iterator<Item = &'a f32>>(mut iter: I) -> f32 {
        let mut total = 1.0f32;
        loop {
            match iter.next() {
                Option::Some(value) => total *= *value,
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

impl<I: Iterator, P: FnMut(&I::Item) -> bool> Iterator for Filter<I, P> {
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
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

impl Iterator for super::ops::Range<i8> {
    type Item = i8;
    fn next(&mut self) -> Option<i8> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1i8;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<i16> {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1i16;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<i32> {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1i32;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<i64> {
    type Item = i64;
    fn next(&mut self) -> Option<i64> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1i64;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<i128> {
    type Item = i128;
    fn next(&mut self) -> Option<i128> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1i128;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<isize> {
    type Item = isize;
    fn next(&mut self) -> Option<isize> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1isize;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<u8> {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1u8;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<u16> {
    type Item = u16;
    fn next(&mut self) -> Option<u16> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1u16;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<u32> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1u32;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<u64> {
    type Item = u64;
    fn next(&mut self) -> Option<u64> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1u64;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<u128> {
    type Item = u128;
    fn next(&mut self) -> Option<u128> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1u128;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}

impl Iterator for super::ops::Range<usize> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.start < self.end {
            let value = self.start;
            self.start = value + 1usize;
            Option::Some(value)
        } else {
            Option::None
        }
    }
}
