const N: usize = 3;
type Row = [u8; N];
fn make() -> Row { [7; N] }
fn main() {
    let values: Row = make();
    let size = values.len();
    println!("{}", size);
    println!("{}", values[2]);
}
