extern "C" { fn native_value() -> u32; }
fn main() { println!("MANIFEST_NATIVE_{}_{}", std::env::var("NATIVE_NONCE").unwrap(), unsafe { native_value() }); }
