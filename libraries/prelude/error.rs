// std::error::Error: the common error-reporting trait.
use super::fmt::{Debug, Display};
use super::option::Option;

pub trait Error: Debug + Display {
    fn source(&self) -> Option<&dyn Error> {
        Option::None
    }
    fn description(&self) -> &str {
        "description() is deprecated; use Display"
    }
}

impl Error for super::primitive::ParseIntError {}
impl Error for super::primitive::ParseFloatError {}
impl Error for super::primitive::ParseBoolError {}
impl Error for super::primitive::ParseCharError {}
impl Error for super::rc::BorrowError {}
impl Error for super::rc::BorrowMutError {}

// String errors own their message, so converting a borrowed string never
// extends the source borrow to the resulting error object.
struct StringError {
    message: super::string::String,
}

impl Debug for StringError {
    fn fmt(&self, f: &mut super::fmt::Formatter<'_>) -> super::fmt::Result {
        <super::string::String as Debug>::fmt(&self.message, f)
    }
}

impl Display for StringError {
    fn fmt(&self, f: &mut super::fmt::Formatter<'_>) -> super::fmt::Result {
        <super::string::String as Display>::fmt(&self.message, f)
    }
}

impl Error for StringError {}

impl<E: Error + 'static> super::convert::From<E> for super::boxed::Box<dyn Error> {
    fn from(value: E) -> Self {
        super::boxed::Box::new(value)
    }
}

impl super::convert::From<super::string::String> for super::boxed::Box<dyn Error> {
    fn from(message: super::string::String) -> Self {
        super::boxed::Box::new(StringError { message })
    }
}

impl<'b> super::convert::From<&'b str> for super::boxed::Box<dyn Error> {
    fn from(message: &'b str) -> Self {
        <Self as super::convert::From<super::string::String>>::from(super::string::String::from(
            message,
        ))
    }
}
