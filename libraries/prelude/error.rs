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
