// Native Result and its ordinary Rust-source methods.
pub enum Result<T, E> {
    Ok(T),
    Err(E),
}

// Scoped to Vec rather than claiming support for arbitrary FromIterator targets.
impl<A, E> super::iter::FromIterator<Result<A, E>> for Result<super::vec::Vec<A>, E> {
    fn from_iter<I: super::iter::IntoIterator<Item = Result<A, E>>>(
        source: I,
    ) -> Result<super::vec::Vec<A>, E> {
        // Drop the input iterator before the partial Vec on an error, as stock does.
        let mut values = super::vec::Vec::new();
        let mut iterator = source.into_iter();
        loop {
            match iterator.next() {
                super::option::Option::Some(Result::Ok(value)) => values.push(value),
                super::option::Option::Some(Result::Err(error)) => return Result::Err(error),
                super::option::Option::None => return Result::Ok(values),
            }
        }
    }
}

// Stop at the first residual without polling the input again, while letting
// the requested success type own the ordinary Sum implementation.
struct SumInput<'a, I, E> {
    iterator: I,
    error: &'a mut super::option::Option<E>,
    failed: bool,
}

impl<'a, I: super::iter::Iterator<Item = Result<U, E>>, U, E> super::iter::Iterator
    for SumInput<'a, I, E>
{
    type Item = U;

    fn next(&mut self) -> super::option::Option<U> {
        if self.failed {
            return super::option::Option::None;
        }
        match self.iterator.next() {
            super::option::Option::Some(Result::Ok(value)) => super::option::Option::Some(value),
            super::option::Option::Some(Result::Err(error)) => {
                self.failed = true;
                *self.error = super::option::Option::Some(error);
                super::option::Option::None
            }
            super::option::Option::None => super::option::Option::None,
        }
    }
}

impl<T: super::iter::Sum<U>, U, E> super::iter::Sum<Result<U, E>> for Result<T, E> {
    fn sum<I: super::iter::Iterator<Item = Result<U, E>>>(iterator: I) -> Result<T, E> {
        let mut error = super::option::Option::None;
        let value = T::sum(SumInput {
            iterator,
            error: &mut error,
            failed: false,
        });
        match error {
            super::option::Option::Some(error) => Result::Err(error),
            super::option::Option::None => Result::Ok(value),
        }
    }
}

impl<T, E> Result<T, E> {
    pub fn is_ok(&self) -> bool {
        match self {
            Result::Ok(_) => true,
            Result::Err(_) => false,
        }
    }

    pub fn is_err(&self) -> bool {
        match self {
            Result::Ok(_) => false,
            Result::Err(_) => true,
        }
    }

    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Result::Ok(value) => value,
            Result::Err(_) => default,
        }
    }

    pub fn unwrap_or_else<F: FnOnce(E) -> T>(self, f: F) -> T {
        match self {
            Result::Ok(value) => value,
            Result::Err(error) => f(error),
        }
    }

    pub fn ok(self) -> super::option::Option<T> {
        match self {
            Result::Ok(value) => super::option::Option::Some(value),
            Result::Err(_) => super::option::Option::None,
        }
    }

    pub fn err(self) -> super::option::Option<E> {
        match self {
            Result::Ok(_) => super::option::Option::None,
            Result::Err(error) => super::option::Option::Some(error),
        }
    }

    pub fn unwrap_err(self) -> E {
        match self {
            Result::Ok(_) => panic!("called `Result::unwrap_err()` on an `Ok` value"),
            Result::Err(error) => error,
        }
    }

    pub fn expect_err(self, msg: &str) -> E {
        match self {
            Result::Ok(_) => {
                super::mem::panic_message(msg);
                panic!("Result::expect_err failed")
            }
            Result::Err(error) => error,
        }
    }

    pub fn or<F>(self, other: Result<T, F>) -> Result<T, F> {
        match self {
            Result::Ok(value) => Result::Ok(value),
            Result::Err(_) => other,
        }
    }

    pub fn and_then<U, F: FnOnce(T) -> Result<U, E>>(self, f: F) -> Result<U, E> {
        match self {
            Result::Ok(value) => f(value),
            Result::Err(error) => Result::Err(error),
        }
    }

    pub fn or_else<F, O: FnOnce(E) -> Result<T, F>>(self, op: O) -> Result<T, F> {
        match self {
            Result::Ok(value) => Result::Ok(value),
            Result::Err(error) => op(error),
        }
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, op: F) -> Result<U, E> {
        match self {
            Result::Ok(value) => Result::Ok(op(value)),
            Result::Err(error) => Result::Err(error),
        }
    }

    pub fn map_err<G, O: FnOnce(E) -> G>(self, op: O) -> Result<T, G> {
        match self {
            Result::Ok(value) => Result::Ok(value),
            Result::Err(error) => Result::Err(op(error)),
        }
    }

    pub fn map_or<U, F: FnOnce(T) -> U>(self, default: U, op: F) -> U {
        match self {
            Result::Ok(value) => op(value),
            Result::Err(_) => default,
        }
    }

    pub fn map_or_else<U, D: FnOnce(E) -> U, F: FnOnce(T) -> U>(self, default: D, op: F) -> U {
        match self {
            Result::Ok(value) => op(value),
            Result::Err(error) => default(error),
        }
    }

    pub fn and<U>(self, other: Result<U, E>) -> Result<U, E> {
        match self {
            Result::Ok(_) => other,
            Result::Err(error) => Result::Err(error),
        }
    }
}

impl<T: super::default::Default, E> Result<T, E> {
    pub fn unwrap_or_default(self) -> T {
        match self {
            Result::Ok(value) => value,
            Result::Err(_) => T::default(),
        }
    }
}

impl<T: Clone, E: Clone> Clone for Result<T, E> {
    fn clone(&self) -> Result<T, E> {
        match self {
            Result::Ok(value) => Result::Ok(value.clone()),
            Result::Err(error) => Result::Err(error.clone()),
        }
    }
}

impl<T: Copy, E: Copy> Copy for Result<T, E> {}

impl<T: super::cmp::PartialEq, E: super::cmp::PartialEq> super::cmp::PartialEq for Result<T, E> {
    fn eq(&self, other: &Result<T, E>) -> bool {
        match self {
            Result::Ok(left) => match other {
                Result::Ok(right) => <T as super::cmp::PartialEq>::eq(left, right),
                Result::Err(_) => false,
            },
            Result::Err(left) => match other {
                Result::Ok(_) => false,
                Result::Err(right) => <E as super::cmp::PartialEq>::eq(left, right),
            },
        }
    }
}

// TODO: require E: Debug and print it once std::fmt is enabled.
impl<T, E> Result<T, E> {
    pub fn unwrap(self) -> T {
        match self {
            Result::Ok(value) => value,
            Result::Err(_) => panic!("called `Result::unwrap()` on an `Err` value"),
        }
    }

    pub fn expect(self, msg: &str) -> T {
        match self {
            Result::Ok(value) => value,
            Result::Err(_) => {
                super::mem::panic_message(msg);
                panic!("Result::expect failed")
            }
        }
    }
}
