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
