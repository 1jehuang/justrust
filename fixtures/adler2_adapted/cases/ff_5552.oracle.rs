extern crate adler2;
fn main() {
let p0: [u8; 5552] = [255; 5552];
let mut state = adler2::Adler32::new();
state.write_slice(&p0);
println!("{}", state.checksum());
}
