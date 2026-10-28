// Native Option and its ordinary Rust-source methods.
pub enum Option<T> {
    None,
    Some(T),
}

impl<T> Option<T> {
    pub fn is_some(&self) -> bool {
        match self {
            Option::Some(_) => true,
            Option::None => false,
        }
    }

    pub fn is_none(&self) -> bool {
        match self {
            Option::Some(_) => false,
            Option::None => true,
        }
    }

    pub fn is_some_and<F: FnOnce(T) -> bool>(self, f: F) -> bool {
        match self {
            Option::Some(value) => f(value),
            Option::None => false,
        }
    }

    pub fn is_none_or<F: FnOnce(T) -> bool>(self, f: F) -> bool {
        match self {
            Option::Some(value) => f(value),
            Option::None => true,
        }
    }

    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Option::Some(value) => value,
            Option::None => default,
        }
    }

    pub fn unwrap_or_else<F: FnOnce() -> T>(self, f: F) -> T {
        match self {
            Option::Some(value) => value,
            Option::None => f(),
        }
    }

    pub fn unwrap(self) -> T {
        match self {
            Option::Some(value) => value,
            Option::None => panic!("called `Option::unwrap()` on a `None` value"),
        }
    }

    pub fn expect(self, msg: &str) -> T {
        match self {
            Option::Some(value) => value,
            Option::None => {
                super::mem::panic_message(msg);
                panic!("Option::expect failed")
            }
        }
    }

    pub fn take(&mut self) -> Option<T> {
        let p = self as *mut Option<T>;
        unsafe {
            let old = *p;
            *p = Option::None;
            old
        }
    }

    pub fn replace(&mut self, value: T) -> Option<T> {
        let p = self as *mut Option<T>;
        unsafe {
            let old = *p;
            *p = Option::Some(value);
            old
        }
    }

    pub fn insert(&mut self, value: T) -> &mut T {
        *self = Option::Some(value);
        match self {
            Option::Some(stored) => stored,
            Option::None => panic!("Option::insert did not store its value"),
        }
    }

    pub fn or(self, other: Option<T>) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some(value),
            Option::None => other,
        }
    }

    pub fn xor(self, other: Option<T>) -> Option<T> {
        match self {
            Option::Some(value) => match other {
                Option::Some(_) => Option::None,
                Option::None => Option::Some(value),
            },
            Option::None => other,
        }
    }

    pub fn ok_or<E>(self, err: E) -> super::result::Result<T, E> {
        match self {
            Option::Some(value) => super::result::Result::Ok(value),
            Option::None => super::result::Result::Err(err),
        }
    }

    pub fn and_then<U, F: FnOnce(T) -> Option<U>>(self, f: F) -> Option<U> {
        match self {
            Option::Some(value) => f(value),
            Option::None => Option::None,
        }
    }

    pub fn or_else<F: FnOnce() -> Option<T>>(self, f: F) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some(value),
            Option::None => f(),
        }
    }

    pub fn ok_or_else<E, F: FnOnce() -> E>(self, f: F) -> super::result::Result<T, E> {
        match self {
            Option::Some(value) => super::result::Result::Ok(value),
            Option::None => super::result::Result::Err(f()),
        }
    }

    pub fn filter<P: FnOnce(&T) -> bool>(self, predicate: P) -> Option<T> {
        match self {
            Option::Some(value) => {
                if predicate(&value) {
                    Option::Some(value)
                } else {
                    Option::None
                }
            }
            Option::None => Option::None,
        }
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Option<U> {
        match self {
            Option::Some(value) => Option::Some(f(value)),
            Option::None => Option::None,
        }
    }

    pub fn iter<'a>(&'a self) -> OptionIter<'a, T> {
        match self {
            Option::Some(value) => OptionIter {
                inner: Option::Some(value),
            },
            Option::None => OptionIter { inner: Option::None },
        }
    }
}

pub struct OptionIter<'a, T> {
    inner: Option<&'a T>,
}

impl<'a, T> super::iter::Iterator for OptionIter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<&'a T> {
        match self.inner {
            Option::Some(value) => {
                self.inner = Option::None;
                Option::Some(value)
            }
            Option::None => Option::None,
        }
    }
}

impl<'a, T> super::iter::IntoIterator for OptionIter<'a, T> {
    type Item = &'a T;
    type IntoIter = OptionIter<'a, T>;
    fn into_iter(self) -> OptionIter<'a, T> {
        self
    }
}

impl<'a, T: Copy> Option<&'a T> {
    pub fn copied(self) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some(*value),
            Option::None => Option::None,
        }
    }
}

impl<'a, T: Copy> Option<&'a mut T> {
    pub fn copied(self) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some(*value),
            Option::None => Option::None,
        }
    }
}

impl<'a, T: Clone> Option<&'a T> {
    pub fn cloned(self) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some((*value).clone()),
            Option::None => Option::None,
        }
    }
}

impl<'a, T: Clone> Option<&'a mut T> {
    pub fn cloned(self) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some((*value).clone()),
            Option::None => Option::None,
        }
    }
}

// TODO: unwrap_or_default once std::default is enabled.


impl<T: Clone> Clone for Option<T> {
    fn clone(&self) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some(value.clone()),
            Option::None => Option::None,
        }
    }
}

impl<T: Copy> Copy for Option<T> {}
