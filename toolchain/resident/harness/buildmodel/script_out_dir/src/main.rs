include!(concat!(env!("OUT_DIR"), "/table.rs"));

fn main() {
    println!("{} rows: {:?}", count(), TABLE);
}
