extern crate adler2;
fn main() {
let p0: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
let mut state = adler2::Adler32::new();
state.write_slice(&p0);
println!("{}", state.checksum());
}
