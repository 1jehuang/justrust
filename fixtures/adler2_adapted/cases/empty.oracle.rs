extern crate adler2;
fn main() {
let p0: [u8; 0] = [];
let mut state = adler2::Adler32::new();
state.write_slice(&p0);
println!("{}", state.checksum());
}
