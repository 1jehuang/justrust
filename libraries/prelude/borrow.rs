//! `std::borrow::Borrow`: a borrowed view of an owned key, used by map
//! lookups such as `HashMap<String, V>::get("key")`.
pub trait Borrow<Borrowed: ?Sized> {
    fn borrow(&self) -> &Borrowed;
}

impl<T: ?Sized> Borrow<T> for T {
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: ?Sized> Borrow<T> for &T {
    fn borrow(&self) -> &T {
        *self
    }
}

/// Conversion from a borrowed value to its independently owned representation.
pub trait ToOwned {
    type Owned: Borrow<Self>;
    fn to_owned(&self) -> Self::Owned;
    fn clone_into(&self, target: &mut Self::Owned) {
        *target = self.to_owned();
    }
}

impl<T: Clone> ToOwned for T {
    type Owned = T;
    fn to_owned(&self) -> T { self.clone() }
}

impl ToOwned for str {
    type Owned = super::string::String;
    fn to_owned(&self) -> super::string::String {
        super::string::String::from(self)
    }
}

impl<T> Borrow<[T]> for super::vec::Vec<T> {
    fn borrow(&self) -> &[T] { self.as_slice() }
}

impl<T: Clone> ToOwned for [T] {
    type Owned = super::vec::Vec<T>;
    fn to_owned(&self) -> super::vec::Vec<T> {
        let mut out = super::vec::Vec::with_capacity(self.len());
        let mut index = 0usize;
        while index < self.len() {
            out.push(self[index].clone());
            index += 1usize;
        }
        out
    }
}

/// A borrowed view which allocates only when ownership or mutation is needed.
pub enum Cow<'a, B: ToOwned + ?Sized> {
    Borrowed(&'a B),
    Owned(B::Owned),
}

impl<'a, B: ToOwned + ?Sized> Cow<'a, B> {
    pub fn into_owned(self) -> B::Owned {
        match self {
            Cow::Borrowed(value) => value.to_owned(),
            Cow::Owned(value) => value,
        }
    }
    pub fn to_mut(&mut self) -> &mut B::Owned {
        if let Cow::Borrowed(value) = self {
            *self = Cow::Owned((*value).to_owned());
        }
        match self {
            Cow::Owned(value) => value,
            Cow::Borrowed(_) => panic!("borrowed Cow was not made owned"),
        }
    }
}

impl<'a, B: ToOwned + ?Sized> super::ops::Deref for Cow<'a, B> {
    type Target = B;
    fn deref(&self) -> &B {
        match self {
            Cow::Borrowed(value) => *value,
            Cow::Owned(value) => value.borrow(),
        }
    }
}

impl<'a, B: ToOwned + ?Sized> Borrow<B> for Cow<'a, B> {
    fn borrow(&self) -> &B {
        super::ops::Deref::deref(self)
    }
}

impl<'a, B: ToOwned + ?Sized> Clone for Cow<'a, B> {
    fn clone(&self) -> Self {
        match self {
            Cow::Borrowed(value) => Cow::Borrowed(*value),
            Cow::Owned(value) => Cow::Owned(value.borrow().to_owned()),
        }
    }
}

impl<'a, B: ToOwned + ?Sized> super::convert::From<&'a B> for Cow<'a, B> {
    fn from(value: &'a B) -> Self { Cow::Borrowed(value) }
}

impl<'a> super::convert::From<super::string::String> for Cow<'a, str> {
    fn from(value: super::string::String) -> Self { Cow::Owned(value) }
}

impl<'a, B: ToOwned + super::fmt::Display + ?Sized> super::fmt::Display for Cow<'a, B> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let value: &B = super::ops::Deref::deref(self);
        super::fmt::Display::fmt(value, f)
    }
}

impl<'a, B: ToOwned + super::fmt::Debug + ?Sized> super::fmt::Debug for Cow<'a, B> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let value: &B = super::ops::Deref::deref(self);
        super::fmt::Debug::fmt(value, f)
    }
}
