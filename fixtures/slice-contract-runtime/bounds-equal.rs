fn read_byte(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

fn read_at_length(bytes: &[u8]) -> u8 {
    read_byte(bytes, bytes.len())
}

fn main() {
    let bytes: [u8; 3] = [1, 2, 3];
    println!("slice-contract-runtime:before");
    let byte = read_at_length(&bytes);
    if byte == 0 {
        println!("slice-contract-runtime:after");
    } else {
        println!("slice-contract-runtime:after");
        println!("slice-contract-runtime:nonzero");
    }
}
