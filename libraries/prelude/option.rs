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
}

impl<T: Clone> Clone for Option<T> {
    fn clone(&self) -> Option<T> {
        match self {
            Option::Some(value) => Option::Some(value.clone()),
            Option::None => Option::None,
        }
    }
}

impl<T: Copy> Copy for Option<T> {}
