mod model;
mod report;
use model::{item::Item, store::Store};
fn main() {
    let mut s = Store::default();
    s.add(Item::new("bolt", 120, 0.05)); s.add(Item::new("nut", 300, 0.02)); s.add(Item::new("gear", 8, 4.5));
    s.take("bolt", 20).unwrap();
    if let Err(e) = s.take("gear", 50) { println!("error: {e}"); }
    print!("{}", report::render(&s));
}
