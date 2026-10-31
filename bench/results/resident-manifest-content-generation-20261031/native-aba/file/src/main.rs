extern "C"{fn native_value()->u32;}
fn main(){println!("NATIVE_{}_{}",std::env::var("NATIVE_NONCE").unwrap(),unsafe{native_value()});}
