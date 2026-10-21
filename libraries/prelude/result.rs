// Native Result and its ordinary Rust-source methods.
pub enum Result<T, E> {
    Ok(T),
    Err(E),
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
