#![no_main]

#[no_mangle]
pub extern "C" fn main(argc: i32, _argv: *const *const u8) -> i32 {
    println!("no_main with std: {}", vec![argc; 3].iter().sum::<i32>());
    0
}
