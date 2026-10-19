// Crate-root attachment of the justrust native prelude library. The compiler
// loads this file after the user crate root, so the library lives in a hidden
// root module. `std::`/`core::` paths and prelude names resolve into it.
mod __justrust_std;
