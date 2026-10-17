fn read_byte(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

fn main() {
    let bytes: [u8; 0] = [];
    println!("slice-contract-runtime:before");
    let byte = read_byte(&bytes, 0);
    if byte == 0 {
        println!("slice-contract-runtime:after");
    } else {
        println!("slice-contract-runtime:after");
        println!("slice-contract-runtime:nonzero");
    }
}
