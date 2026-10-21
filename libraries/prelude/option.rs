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

    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Option::Some(value) => value,
            Option::None => default,
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

    pub fn insert(&mut self, value: T) {
        *self = Option::Some(value);
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

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Option<U> {
        match self {
            Option::Some(value) => Option::Some(f(value)),
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
