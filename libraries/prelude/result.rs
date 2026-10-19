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
}
