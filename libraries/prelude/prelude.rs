// Names injected into every user module scope. User items and explicit imports
// shadow these. Glob imports remain unsupported.
pub mod v1 {
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::vec::Vec;
    pub use super::super::string::String;
}
