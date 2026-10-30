//! rapidhash and brotli-decompressor test their command line tools, which cargo builds for
//! the tests and names in `CARGO_BIN_EXE_<name>`. ra-jit did not know the variable, and ran a
//! program named for the lack of it.

use std::process::Command;

#[test]
fn speaks() {
    let said = Command::new(env!("CARGO_BIN_EXE_speaker")).output().expect("the program runs");
    assert_eq!(String::from_utf8_lossy(&said.stdout), "spoken\n");
}
