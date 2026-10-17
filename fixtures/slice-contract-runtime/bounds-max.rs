fn read_byte(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

fn main() {
    let bytes: [u8; 3] = [1, 2, 3];
    let index: usize = 18446744073709551615;
    println!("slice-contract-runtime:before");
    let byte = read_byte(&bytes, index);
    if byte == 0 {
        println!("slice-contract-runtime:after");
    } else {
        println!("slice-contract-runtime:after");
        println!("slice-contract-runtime:nonzero");
    }
}
