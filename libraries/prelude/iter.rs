//! Native iterator protocols and ordinary source adapters.
use super::option::Option;

pub trait Iterator: Sized {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;

    fn enumerate(self) -> Enumerate<Self> {
        Enumerate { iterator: self, count: 0usize }
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

pub trait IntoIterator {
    type Item;
    type IntoIter: Iterator<Item = Self::Item>;
    fn into_iter(self) -> Self::IntoIter;
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
