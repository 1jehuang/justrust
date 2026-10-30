// The crate's documentation is all the README, from a line
// other than the first.
#![doc = include_str!("../README.md")]

pub mod a {
    //! One string, a fence in it once.
    #![doc = "
```rust
assert!(true);
```
"]
}

#[doc = "
```
assert!(true);
```
"]
pub fn twice_in_a_string() {}

/**
```
assert!(true);
```

```
assert!(true);
```
*/
pub fn block() {}

/// x
#[doc = "y"]
/**
```
assert!(true);
```
*/
pub fn mixed_block() {}

/// x
#[doc = "y\nz"]
/// ```should_panic
/// panic!();
/// ```
pub fn mixed_lines() {}
