#![no_main]

fn sum(argc: i32) -> i32 {
    vec![argc; 3].iter().sum()
}

#[no_mangle]
pub extern "C" fn main(argc: i32, _argv: *const *const u8) -> i32 {
    println!("no_main with std: {}", sum(argc));
    0
}
