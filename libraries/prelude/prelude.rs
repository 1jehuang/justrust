// Names injected into every user module scope. User items, imports and glob
// imports shadow these.
pub mod v1 {
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
}
